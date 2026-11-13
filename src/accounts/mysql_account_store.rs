use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::Utc;
use mysql::prelude::Queryable;
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::models::{
    AccountOverview, AuthUser, BillingConfig, CreditChangeView, MICRO_CREDITS, RequestLog,
    Reservation, Role, TransactionView, UserView,
};
use super::mysql_store::{MySqlStore, Params, mysql_error};

pub use super::store::AccountError;

pub struct AccountStore {
    db: MySqlStore,
}

impl AccountStore {
    pub fn open(url: &str) -> Result<Self, AccountError> {
        Ok(Self {
            db: MySqlStore::open(url)?,
        })
    }

    pub fn has_users(&self) -> Result<bool, AccountError> {
        Ok(self
            .db
            .first::<i64>("SELECT COUNT(*) FROM users", vec![])?
            .unwrap_or(0)
            > 0)
    }

    pub fn ensure_super_admin(&self, admin_key: &str) -> Result<(), AccountError> {
        let exists: Option<i64> = self.db.first(
            "SELECT COUNT(*) FROM users WHERE role = 'super_admin'",
            vec![],
        )?;
        if exists.unwrap_or(0) == 0 {
            self.db.execute("INSERT INTO users (username, password_hash, api_key_hash, key_prefix, role, created_at) VALUES ('superadmin', ?, ?, ?, 'super_admin', ?)", vec![hash_password(admin_key)?.into(), key_hash(admin_key).into(), key_prefix(admin_key).into(), mysql::Value::from(now())])?;
        }
        Ok(())
    }

    pub fn super_admin(&self) -> Result<AuthUser, AccountError> {
        self.first_user("SELECT id, username, role, balance_micro, enabled, parent_id FROM users WHERE role = 'super_admin' ORDER BY id LIMIT 1", vec![])?.filter(|user| user.enabled).ok_or_else(|| AccountError::Internal("超级管理员未初始化或已禁用".into()))
    }

    pub fn authenticate_key(&self, key: &str) -> Result<Option<AuthUser>, AccountError> {
        Ok(self.first_user("SELECT id, username, role, balance_micro, enabled, parent_id FROM users WHERE api_key_hash = ?", vec![mysql::Value::from(key_hash(key))])?.filter(|user| user.enabled))
    }

    pub fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<AuthUser>, AccountError> {
        let result: Option<(String, String, String, i64, i64, Option<i64>)> = self.db.first("SELECT username, password_hash, role, balance_micro, enabled, parent_id FROM users WHERE username = ?", vec![username.trim().into()])?;
        match result {
            Some((name, hash, role, balance, enabled, parent_id))
                if enabled != 0 && verify_password(password, &hash).unwrap_or(false) =>
            {
                Ok(Some(AuthUser {
                    id: self.user_id(&name)?,
                    username: name,
                    role: Role::parse(&role).unwrap_or(Role::User),
                    balance_micro: balance,
                    enabled: true,
                    parent_id,
                }))
            }
            _ => Ok(None),
        }
    }

    fn user_id(&self, username: &str) -> Result<i64, AccountError> {
        self.db
            .first(
                "SELECT id FROM users WHERE username = ?",
                vec![mysql::Value::from(username)],
            )?
            .ok_or(AccountError::NotFound)
    }

    pub fn create_web_session(&self, user: &AuthUser) -> Result<String, AccountError> {
        let token = Uuid::new_v4().simple().to_string() + &Uuid::new_v4().simple().to_string();
        self.db.execute(
            "DELETE FROM web_sessions WHERE expires_at <= ?",
            vec![mysql::Value::from(now())],
        )?;
        self.db.execute(
            "INSERT INTO web_sessions VALUES (?, ?, ?)",
            vec![
                mysql::Value::from(key_hash(&token)),
                mysql::Value::from(user.id),
                (Utc::now() + chrono::Duration::hours(12))
                    .to_rfc3339()
                    .into(),
            ],
        )?;
        Ok(token)
    }

    pub fn authenticate_web_session(&self, token: &str) -> Result<Option<AuthUser>, AccountError> {
        let id: Option<i64> = self.db.first(
            "SELECT user_id FROM web_sessions WHERE token_hash = ? AND expires_at > ?",
            vec![
                mysql::Value::from(key_hash(token)),
                mysql::Value::from(now()),
            ],
        )?;
        match id {
            Some(id) => {
                let user = self.authenticate_session(id)?;
                Ok(user.enabled.then_some(user))
            }
            None => Ok(None),
        }
    }

    pub fn revoke_web_session(&self, token: &str) -> Result<(), AccountError> {
        self.db.execute(
            "DELETE FROM web_sessions WHERE token_hash = ?",
            vec![mysql::Value::from(key_hash(token))],
        )?;
        Ok(())
    }

    pub fn authenticate_session(&self, id: i64) -> Result<AuthUser, AccountError> {
        let user = self.first_user("SELECT id, username, role, balance_micro, enabled, parent_id FROM users WHERE id = ?", vec![mysql::Value::from(id)])?.ok_or(AccountError::NotFound)?;
        if user.enabled {
            Ok(user)
        } else {
            Err(AccountError::Forbidden("账户已被禁用".into()))
        }
    }

    pub fn change_password(
        &self,
        user: &AuthUser,
        current_password: &str,
        new_password: &str,
    ) -> Result<String, AccountError> {
        validate_password(new_password)?;
        let current: String = self
            .db
            .first(
                "SELECT password_hash FROM users WHERE id = ?",
                vec![mysql::Value::from(user.id)],
            )?
            .ok_or(AccountError::NotFound)?;
        if !verify_password(current_password, &current)? {
            return Err(AccountError::Forbidden("当前密码不正确".into()));
        }
        let api_key = generate_api_key();
        self.db.execute(
            "UPDATE users SET password_hash = ?, api_key_hash = ?, key_prefix = ? WHERE id = ?",
            vec![
                hash_password(new_password)?.into(),
                key_hash(&api_key).into(),
                key_prefix(&api_key).into(),
                mysql::Value::from(user.id),
            ],
        )?;
        Ok(api_key)
    }

    pub fn create_user(
        &self,
        actor: &AuthUser,
        username: &str,
        password: &str,
        role: Role,
        initial_balance: f64,
    ) -> Result<(UserView, String), AccountError> {
        if !actor.role.can_create(role) {
            return Err(AccountError::Forbidden("无权创建该角色的账户".into()));
        }
        let username = username.trim();
        if username.len() < 3
            || username.len() > 40
            || !username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
        {
            return Err(AccountError::InvalidInput(
                "用户名需为 3-40 位字母、数字或 _ - . @".into(),
            ));
        }
        validate_password(password)?;
        let amount = credits_to_micro(initial_balance)?;
        let api_key = generate_api_key();
        let result = self.db.execute("INSERT INTO users (username, password_hash, api_key_hash, key_prefix, role, balance_micro, created_at, parent_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)", vec![mysql::Value::from(username), hash_password(password)?.into(), key_hash(&api_key).into(), key_prefix(&api_key).into(), role.as_str().into(), mysql::Value::from(amount), mysql::Value::from(now()), mysql::Value::from(actor.id)]);
        if let Err(error) = result {
            return if error.to_string().contains("Duplicate entry") {
                Err(AccountError::Conflict("用户名已存在".into()))
            } else {
                Err(error)
            };
        }
        let id = self.user_id(username)?;
        if amount > 0 {
            self.insert_transaction(
                id,
                Some(actor.id),
                "initial",
                amount,
                amount,
                "settled",
                Some("开户初始积分"),
                None,
            )?;
        }
        Ok((self.get_user_view(id)?, api_key))
    }

    pub fn list_users(&self, actor: &AuthUser) -> Result<Vec<UserView>, AccountError> {
        if !actor.is_manager() {
            return Err(AccountError::Forbidden("仅管理员可以查看用户列表".into()));
        }
        let rows: Vec<(i64, String, String, i64, i64, Option<i64>, String, String)> = self.db.query("SELECT id, username, role, balance_micro, enabled, parent_id, key_prefix, created_at FROM users ORDER BY id", vec![])?;
        let visible_ids = self.manageable_account_ids(actor)?;
        let mut users = Vec::new();
        for (id, username, role, balance_micro, enabled, parent_id, key_prefix, created_at) in rows
        {
            if actor.role != Role::SuperAdmin && !visible_ids.contains(&id) {
                continue;
            }
            let role = Role::parse(&role).unwrap_or(Role::User);
            let mut user = UserView {
                id,
                username,
                role,
                parent_id,
                balance_micro,
                enabled: enabled != 0,
                key_prefix,
                created_at,
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_spent_micro: 0,
            };
            self.add_usage(&mut user)?;
            users.push(user);
        }
        Ok(users)
    }

    pub fn get_user_view(&self, id: i64) -> Result<UserView, AccountError> {
        let row: Option<(i64, String, String, i64, i64, Option<i64>, String, String)> = self.db.first("SELECT id, username, role, balance_micro, enabled, parent_id, key_prefix, created_at FROM users WHERE id = ?", vec![mysql::Value::from(id)])?;
        let (id, username, role, balance_micro, enabled, parent_id, key_prefix, created_at) =
            row.ok_or(AccountError::NotFound)?;
        let mut user = UserView {
            id,
            username,
            role: Role::parse(&role).unwrap_or(Role::User),
            parent_id,
            balance_micro,
            enabled: enabled != 0,
            key_prefix,
            created_at,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_spent_micro: 0,
        };
        self.add_usage(&mut user)?;
        Ok(user)
    }

    fn add_usage(&self, user: &mut UserView) -> Result<(), AccountError> {
        let usage: (i64, i64, i64) = self.db.first("SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(-amount_micro), 0) FROM credit_transactions WHERE user_id = ? AND kind = 'charge' AND status = 'settled'", vec![mysql::Value::from(user.id)])?.unwrap_or((0, 0, 0));
        user.total_input_tokens = usage.0;
        user.total_output_tokens = usage.1;
        user.total_spent_micro = usage.2;
        Ok(())
    }

    pub fn update_user(
        &self,
        actor: &AuthUser,
        id: i64,
        enabled: Option<bool>,
        role: Option<Role>,
    ) -> Result<UserView, AccountError> {
        let target = self.get_user_view(id)?;
        if actor.role == Role::Admin && !self.is_descendant(actor.id, id)? {
            return Err(AccountError::Forbidden(
                "只能管理自己及下级代理的账户".into(),
            ));
        }
        if actor.id == id && (enabled == Some(false) || role.is_some_and(|r| r != actor.role)) {
            return Err(AccountError::InvalidInput(
                "不能禁用自己的账户或修改自己的角色".into(),
            ));
        }
        if !actor.role.can_manage(target.role) || role.is_some_and(|r| !actor.role.can_manage(r)) {
            return Err(AccountError::Forbidden("无权修改该账户或角色".into()));
        }
        self.db.execute("UPDATE users SET enabled = COALESCE(?, enabled), role = COALESCE(?, role) WHERE id = ?", vec![enabled.map(|v| v as i64).into(), role.map(|r| r.as_str()).into(), mysql::Value::from(id)])?;
        self.get_user_view(id)
    }

    pub fn regenerate_key(&self, actor: &AuthUser, id: i64) -> Result<String, AccountError> {
        let target = self.get_user_view(id)?;
        if actor.role == Role::Admin && actor.id != id && !self.is_descendant(actor.id, id)? {
            return Err(AccountError::Forbidden(
                "只能管理自己及下级代理的账户".into(),
            ));
        }
        if actor.id != id && !actor.role.can_manage(target.role) {
            return Err(AccountError::Forbidden("无权重置该账户的 API Key".into()));
        }
        let api_key = generate_api_key();
        self.db.execute(
            "UPDATE users SET api_key_hash = ?, key_prefix = ? WHERE id = ?",
            vec![
                key_hash(&api_key).into(),
                key_prefix(&api_key).into(),
                mysql::Value::from(id),
            ],
        )?;
        Ok(api_key)
    }

    pub fn adjust_credits(
        &self,
        actor: &AuthUser,
        id: i64,
        amount: f64,
        note: Option<&str>,
    ) -> Result<UserView, AccountError> {
        if actor.id == id {
            return Err(AccountError::Forbidden(
                "不能调整自己的积分，请由其他管理员操作".into(),
            ));
        }
        let target = self.get_user_view(id)?;
        if actor.role == Role::Admin && !self.is_descendant(actor.id, id)? {
            return Err(AccountError::Forbidden(
                "只能管理自己及下级代理的账户".into(),
            ));
        }
        if !actor.role.can_manage(target.role) {
            return Err(AccountError::Forbidden("无权调整该账户的积分".into()));
        }
        let delta = credits_to_micro(amount)?;
        if delta == 0 {
            return Err(AccountError::InvalidInput("调整数量不能为零".into()));
        }
        self.db.transaction(|tx| {
            tx.exec_drop("UPDATE users SET balance_micro = balance_micro + ? WHERE id = ? AND balance_micro + ? >= 0", (delta, id, delta)).map_err(mysql_error)?;
            if tx.affected_rows() == 0 { return Err(AccountError::InsufficientCredits { required_micro: -delta, balance_micro: target.balance_micro }); }
            let balance: i64 = tx.exec_first("SELECT balance_micro FROM users WHERE id = ?", (id,)).map_err(mysql_error)?.ok_or(AccountError::NotFound)?;
            tx.exec_drop("INSERT INTO credit_transactions (id, user_id, actor_id, kind, amount_micro, balance_after_micro, status, note, created_at, completed_at) VALUES (?, ?, ?, 'adjustment', ?, ?, 'settled', ?, ?, ?)", (Uuid::new_v4().to_string(), id, actor.id, delta, balance, note, now(), now())).map_err(mysql_error)?;
            Ok(())
        })?;
        self.get_user_view(id)
    }

    pub fn get_billing_config(&self) -> Result<BillingConfig, AccountError> {
        let (input_per_million, output_per_million) = self
            .db
            .first(
                "SELECT input_per_million, output_per_million FROM billing_config WHERE id = 1",
                vec![],
            )?
            .ok_or_else(|| AccountError::Internal("计费配置不存在".into()))?;
        Ok(BillingConfig {
            input_per_million,
            output_per_million,
        })
    }

    pub fn set_billing_config(&self, config: &BillingConfig) -> Result<(), AccountError> {
        for price in [config.input_per_million, config.output_per_million] {
            if !price.is_finite() || !(0.0..=1_000_000_000.0).contains(&price) {
                return Err(AccountError::InvalidInput(
                    "单价必须在 0 至 1,000,000,000 积分/百万 token 之间".into(),
                ));
            }
        }
        self.db.execute(
            "UPDATE billing_config SET input_per_million = ?, output_per_million = ? WHERE id = 1",
            vec![
                config.input_per_million.into(),
                config.output_per_million.into(),
            ],
        )?;
        Ok(())
    }

    pub fn overview(&self, user: &AuthUser) -> Result<AccountOverview, AccountError> {
        let user = self.authenticate_session(user.id)?;
        let (total_input_tokens, total_output_tokens, total_spent_micro): (i64, i64, i64) = self.db.first("SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(-amount_micro), 0) FROM credit_transactions WHERE user_id = ? AND kind = 'charge' AND status = 'settled'", vec![mysql::Value::from(user.id)])?.unwrap_or((0, 0, 0));
        let pending_count = self
            .db
            .first(
                "SELECT COUNT(*) FROM credit_transactions WHERE user_id = ? AND status = 'pending'",
                vec![mysql::Value::from(user.id)],
            )?
            .unwrap_or(0);
        Ok(AccountOverview {
            user,
            total_input_tokens,
            total_output_tokens,
            total_spent_micro,
            pending_count,
            billing: self.get_billing_config()?,
        })
    }

    pub fn ensure_transaction_access(
        &self,
        actor: &AuthUser,
        requested_user: Option<i64>,
    ) -> Result<(), AccountError> {
        if actor.role == Role::SuperAdmin {
            return Ok(());
        }
        match requested_user {
            Some(user_id) if actor.id == user_id || self.is_descendant(actor.id, user_id)? => {
                Ok(())
            }
            Some(_) => Err(AccountError::Forbidden(
                "只能查看自己及下级代理的流水".into(),
            )),
            None => Ok(()),
        }
    }

    pub fn list_transactions(
        &self,
        actor: &AuthUser,
        requested_user: Option<i64>,
        limit: u32,
    ) -> Result<Vec<TransactionView>, AccountError> {
        self.ensure_transaction_access(actor, requested_user)?;
        let user_id = if actor.is_manager() {
            requested_user
        } else {
            Some(actor.id)
        };
        let sql = "SELECT t.id, t.user_id, u.username, t.kind, t.amount_micro, t.balance_after_micro, t.model, t.input_tokens, t.output_tokens, t.reserved_micro, t.status, t.note, t.created_at, t.completed_at, t.request_id, t.path, t.stream, t.balance_before_micro, t.reserved_balance_micro, COALESCE(r.amount_micro, 0), t.http_status, t.error_message, t.duration_ms, t.first_token_ms FROM credit_transactions t JOIN users u ON u.id = t.user_id LEFT JOIN credit_transactions r ON r.request_id = t.request_id AND r.kind IN ('refund', 'release') WHERE t.kind IN ('reserve', 'charge', 'error') ";
        let rows: Vec<mysql::Row> = if let Some(id) = user_id {
            self.db.map(
                &format!("{sql} AND t.user_id = ? ORDER BY t.created_at DESC, t.id DESC LIMIT ?"),
                vec![mysql::Value::from(id), mysql::Value::from(limit.min(500))],
                |row| row,
            )?
        } else {
            self.db.map(
                &format!("{sql} ORDER BY t.created_at DESC, t.id DESC LIMIT ?"),
                vec![mysql::Value::from(limit.min(500))],
                |row| row,
            )?
        };
        Ok(rows.into_iter().map(transaction_row).collect())
    }

    pub fn usage_stats(
        &self,
        actor: &AuthUser,
        start: &str,
        end: &str,
        group_by: &str,
        requested_user: Option<i64>,
        limit: u32,
    ) -> Result<crate::admin::usage_types::UsageStatsResponse, AccountError> {
        self.ensure_transaction_access(actor, requested_user)?;
        let visible = self.manageable_account_ids(actor)?;
        let group_expression = match group_by {
            "day" => {
                "DATE_FORMAT(COALESCE(t.completed_at, t.created_at) + INTERVAL 8 HOUR, '%Y-%m-%d')"
            }
            "user" => "u.username",
            "model" => "COALESCE(t.model, '未知模型')",
            "status" => {
                "CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN '失败' ELSE '成功' END"
            }
            _ => return Err(AccountError::InvalidInput("不支持的统计维度".into())),
        };
        let scope = if actor.role == Role::SuperAdmin {
            String::new()
        } else if let Some(user_id) = requested_user {
            format!(" AND t.user_id = {}", user_id as i64)
        } else {
            let ids = visible
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!(" AND t.user_id IN ({ids})")
        };
        let filter = format!(
            "WHERE t.kind IN ('charge', 'error') AND t.status = 'settled' AND COALESCE(t.completed_at, t.created_at) >= ? AND COALESCE(t.completed_at, t.created_at) < ?{scope}"
        );
        let rows: Vec<(String, i64, i64, i64, i64, i64, i64, i64, Option<f64>, Option<f64>)> = self.db.query(
            &format!("SELECT {group_expression}, COUNT(*), SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 0 ELSE 1 END), SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 1 ELSE 0 END), SUM(t.input_tokens), SUM(t.output_tokens), SUM(t.input_tokens + t.output_tokens), SUM(GREATEST(-t.amount_micro, 0)), AVG(t.duration_ms), AVG(t.first_token_ms) FROM credit_transactions t JOIN users u ON u.id = t.user_id {filter} GROUP BY {group_expression} ORDER BY SUM(t.input_tokens + t.output_tokens) DESC, COUNT(*) DESC LIMIT ?"),
            vec![start.into(), end.into(), mysql::Value::from(limit.min(200))],
        )?;
        let totals = self.db.first::<(i64, i64, i64, i64, i64, i64, i64, i64)>(
            &format!("SELECT COUNT(*), COALESCE(SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 0 ELSE 1 END), 0), COALESCE(SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 1 ELSE 0 END), 0), COUNT(DISTINCT t.user_id), COALESCE(SUM(t.input_tokens), 0), COALESCE(SUM(t.output_tokens), 0), COALESCE(SUM(t.input_tokens + t.output_tokens), 0), COALESCE(SUM(GREATEST(-t.amount_micro, 0)), 0) FROM credit_transactions t JOIN users u ON u.id = t.user_id {filter}"),
            vec![start.into(), end.into()],
        )?.unwrap_or_default();
        let groups = rows
            .into_iter()
            .map(
                |(
                    key,
                    requests,
                    successful_requests,
                    failed_requests,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    spent_micro,
                    average_duration_ms,
                    average_first_token_ms,
                )| crate::admin::usage_types::UsageGroup {
                    key,
                    requests,
                    successful_requests,
                    failed_requests,
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    spent_micro,
                    average_duration_ms: average_duration_ms.map(|value| value.round() as i64),
                    average_first_token_ms: average_first_token_ms
                        .map(|value| value.round() as i64),
                },
            )
            .collect::<Vec<_>>();
        Ok(crate::admin::usage_types::UsageStatsResponse {
            start_date: start.into(),
            end_date: end.into(),
            time_zone: "Asia/Shanghai".into(),
            group_by: group_by.into(),
            total_requests: totals.0,
            successful_requests: totals.1,
            failed_requests: totals.2,
            active_users: totals.3,
            input_tokens: totals.4,
            output_tokens: totals.5,
            total_tokens: totals.6,
            spent_micro: totals.7,
            groups,
        })
    }

    pub fn reserve(
        &self,
        user: &AuthUser,
        model: &str,
        estimated_input: i64,
        output_limit: i64,
    ) -> Result<Option<Reservation>, AccountError> {
        if user.is_manager() {
            return Ok(None);
        }
        if estimated_input < 0 || output_limit <= 0 {
            return Err(AccountError::InvalidInput("max_tokens 必须为正数".into()));
        }
        let billing = self.get_billing_config()?;
        let amount = billing.cost_micro(estimated_input, output_limit);
        let id = format!("req_{}", Uuid::new_v4().simple());
        self.db.transaction(|tx| {
            tx.exec_drop("UPDATE users SET balance_micro = balance_micro - ? WHERE id = ? AND enabled = 1 AND balance_micro >= ?", (amount, user.id, amount)).map_err(mysql_error)?;
            if tx.affected_rows() == 0 { let balance = tx.exec_first("SELECT balance_micro FROM users WHERE id = ?", (user.id,)).map_err(mysql_error)?.ok_or(AccountError::NotFound)?; return Err(AccountError::InsufficientCredits { required_micro: amount, balance_micro: balance }); }
            let balance: i64 = tx.exec_first("SELECT balance_micro FROM users WHERE id = ?", (user.id,)).map_err(mysql_error)?.ok_or(AccountError::NotFound)?;
            tx.exec_drop("INSERT INTO credit_transactions (id, user_id, kind, amount_micro, balance_after_micro, model, input_tokens, output_tokens, reserved_micro, input_rate, output_rate, status, created_at, request_id, balance_before_micro, reserved_balance_micro) VALUES (?, ?, 'reserve', ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?)", vec![
                mysql::Value::from(id.clone()), mysql::Value::from(user.id), mysql::Value::from(-amount), mysql::Value::from(balance),
                mysql::Value::from(model), mysql::Value::from(estimated_input), mysql::Value::from(output_limit), mysql::Value::from(amount),
                mysql::Value::from(billing.input_per_million), mysql::Value::from(billing.output_per_million), mysql::Value::from(now()),
                mysql::Value::from(id.clone()), mysql::Value::from(balance + amount), mysql::Value::from(balance),
            ]).map_err(mysql_error)?;
            Ok(Some(Reservation { id, user_id: user.id, amount_micro: amount, input_tokens: estimated_input }))
        })
    }

    pub fn settle(
        &self,
        reservation: &Reservation,
        success: bool,
        input_tokens: i64,
        output_tokens: i64,
        log: Option<&RequestLog>,
    ) -> Result<(), AccountError> {
        self.db.transaction(|tx| {
            tx.exec_drop("UPDATE credit_transactions SET status = 'processing' WHERE id = ? AND status = 'pending'", (&reservation.id,)).map_err(mysql_error)?;
            if tx.affected_rows() == 0 { return Ok(()); }
            let (input_rate, output_rate): (f64, f64) = tx.exec_first("SELECT input_rate, output_rate FROM credit_transactions WHERE id = ?", (&reservation.id,)).map_err(mysql_error)?.ok_or(AccountError::NotFound)?;
            let actual = if success { BillingConfig { input_per_million: input_rate, output_per_million: output_rate }.cost_micro(input_tokens, output_tokens).min(reservation.amount_micro) } else { 0 };
            let refund = reservation.amount_micro - actual;
            if refund > 0 { tx.exec_drop("UPDATE users SET balance_micro = balance_micro + ? WHERE id = ?", (refund, reservation.user_id)).map_err(mysql_error)?; }
            let balance: i64 = tx.exec_first("SELECT balance_micro FROM users WHERE id = ?", (reservation.user_id,)).map_err(mysql_error)?.ok_or(AccountError::NotFound)?;
            let reserved_balance_micro: Option<i64> = tx.exec_first("SELECT reserved_balance_micro FROM credit_transactions WHERE id = ?", (&reservation.id,)).map_err(mysql_error)?;
            let balance_before_micro = reserved_balance_micro.map(|b| b + reservation.amount_micro);
            tx.exec_drop("UPDATE credit_transactions SET kind = 'charge', amount_micro = ?, balance_after_micro = ?, input_tokens = ?, output_tokens = ?, status = 'settled', completed_at = ?, request_id = ?, path = COALESCE(?, path), model = COALESCE(?, model), stream = COALESCE(?, stream), balance_before_micro = COALESCE(?, balance_before_micro), reserved_balance_micro = COALESCE(?, reserved_balance_micro), http_status = ?, error_message = ?, duration_ms = ?, first_token_ms = ? WHERE id = ?", vec![
                mysql::Value::from(-actual), mysql::Value::from(balance), mysql::Value::from(input_tokens), mysql::Value::from(output_tokens),
                mysql::Value::from(now()), mysql::Value::from(reservation.id.clone()),
                mysql::Value::from(log.map(|l| l.path.as_str())), mysql::Value::from(log.map(|l| l.model.as_str())),
                mysql::Value::from(log.map(|l| l.stream as i64)), mysql::Value::from(balance_before_micro), mysql::Value::from(reserved_balance_micro),
                mysql::Value::from(log.map(|l| l.http_status)), mysql::Value::from(log.and_then(|l| l.error_message.as_deref())),
                mysql::Value::from(log.map(|l| l.duration_ms)), mysql::Value::from(log.and_then(|l| l.first_token_ms)), mysql::Value::from(reservation.id.clone()),
            ]).map_err(mysql_error)?;
            tx.exec_drop("INSERT INTO credit_transactions (id, user_id, actor_id, kind, amount_micro, balance_after_micro, status, note, created_at, completed_at, request_id) VALUES (?, ?, NULL, ?, ?, ?, 'settled', ?, ?, ?, ?)", (Uuid::new_v4().to_string(), reservation.user_id, if success { "refund" } else { "release" }, refund, balance, Some(if success { "预授权未使用部分返还" } else { "请求失败，预授权全额返还" }), now(), now(), &reservation.id)).map_err(mysql_error)?;
            Ok(())
        })
    }

    pub fn refund(&self, reservation: &Reservation, log: Option<&RequestLog>) {
        if let Err(error) = self.settle(reservation, false, 0, 0, log) {
            tracing::error!(reservation = %reservation.id, "释放预授权积分失败: {}", error);
        }
    }

    pub fn record_rejected_request(
        &self,
        user: &AuthUser,
        path: &str,
        model: &str,
        stream: bool,
        http_status: u16,
        error_message: &str,
        duration_ms: i64,
    ) {
        let id = format!("req_{}", Uuid::new_v4().simple());
        if let Err(error) = self.db.execute("INSERT INTO credit_transactions (id, user_id, kind, amount_micro, balance_after_micro, model, reserved_micro, status, created_at, completed_at, request_id, path, stream, http_status, error_message, duration_ms) VALUES (?, ?, 'error', 0, ?, ?, 0, 'settled', ?, ?, ?, ?, ?, ?, ?, ?)", vec![id.clone().into(), mysql::Value::from(user.id), user.balance_micro.into(), mysql::Value::from(model), mysql::Value::from(now()), mysql::Value::from(now()), mysql::Value::from(id), mysql::Value::from(path), (stream as i64).into(), (http_status as i64).into(), mysql::Value::from(error_message), mysql::Value::from(duration_ms)]) { tracing::error!("记录请求错误日志失败: {}", error); }
    }

    pub fn list_credit_changes(
        &self,
        actor: &AuthUser,
        requested_user: Option<i64>,
        limit: u32,
    ) -> Result<Vec<CreditChangeView>, AccountError> {
        self.ensure_transaction_access(actor, requested_user)?;
        let user_id = if actor.is_manager() {
            match requested_user {
                Some(_) => requested_user,
                None if actor.role == Role::SuperAdmin => None,
                None => Some(actor.id),
            }
        } else {
            Some(actor.id)
        };
        let sql = "SELECT kind, amount_micro, balance_after_micro, note, created_at, completed_at FROM credit_transactions WHERE request_id IS NULL AND kind IN ('initial', 'adjustment')";
        let rows: Vec<(String, i64, i64, Option<String>, String, Option<String>)> =
            if let Some(id) = user_id {
                self.db.query(
                    &format!("{sql} AND user_id = ? ORDER BY created_at DESC, id DESC LIMIT ?"),
                    vec![mysql::Value::from(id), mysql::Value::from(limit.min(500))],
                )?
            } else {
                self.db.query(
                    &format!("{sql} ORDER BY created_at DESC, id DESC LIMIT ?"),
                    vec![mysql::Value::from(limit.min(500))],
                )?
            };
        Ok(rows
            .into_iter()
            .map(
                |(kind, amount_micro, balance_after_micro, note, created_at, completed_at)| {
                    CreditChangeView {
                        kind,
                        amount_micro,
                        balance_after_micro,
                        note,
                        created_at,
                        completed_at,
                    }
                },
            )
            .collect())
    }

    pub fn manageable_account_ids(
        &self,
        actor: &AuthUser,
    ) -> Result<std::collections::HashSet<i64>, AccountError> {
        if actor.role == Role::SuperAdmin {
            return Ok(std::collections::HashSet::new());
        }
        let relations: Vec<(i64, Option<i64>)> =
            self.db.query("SELECT id, parent_id FROM users", vec![])?;
        let mut children = std::collections::HashMap::<i64, Vec<i64>>::new();
        for (id, parent_id) in relations {
            if let Some(parent_id) = parent_id {
                children.entry(parent_id).or_default().push(id);
            }
        }
        let mut visible = std::collections::HashSet::from([actor.id]);
        let mut pending = vec![actor.id];
        while let Some(parent_id) = pending.pop() {
            if let Some(child_ids) = children.get(&parent_id) {
                for id in child_ids {
                    if visible.insert(*id) {
                        pending.push(*id);
                    }
                }
            }
        }
        Ok(visible)
    }

    fn is_descendant(&self, ancestor_id: i64, target_id: i64) -> Result<bool, AccountError> {
        if ancestor_id == target_id {
            return Ok(true);
        }
        let mut current_id = target_id;
        let mut visited = std::collections::HashSet::new();
        while visited.insert(current_id) {
            let parent_id: Option<Option<i64>> = self.db.first(
                "SELECT parent_id FROM users WHERE id = ?",
                vec![mysql::Value::from(current_id)],
            )?;
            match parent_id.flatten() {
                Some(id) if id == ancestor_id => return Ok(true),
                Some(id) => current_id = id,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    pub fn can_manage_credential(
        &self,
        actor: &AuthUser,
        owner_id: Option<i64>,
    ) -> Result<bool, AccountError> {
        if actor.role == Role::SuperAdmin {
            return Ok(true);
        }
        match owner_id {
            Some(owner_id) if actor.is_manager() => {
                Ok(actor.id == owner_id || self.is_descendant(actor.id, owner_id)?)
            }
            _ => Ok(false),
        }
    }

    fn first_user(&self, sql: &str, params: Params) -> Result<Option<AuthUser>, AccountError> {
        let row: Option<(i64, String, String, i64, i64, Option<i64>)> =
            self.db.first(sql, params)?;
        Ok(row.map(
            |(id, username, role, balance_micro, enabled, parent_id)| AuthUser {
                id,
                username,
                role: Role::parse(&role).unwrap_or(Role::User),
                balance_micro,
                enabled: enabled != 0,
                parent_id,
            },
        ))
    }

    fn insert_transaction(
        &self,
        user_id: i64,
        actor_id: Option<i64>,
        kind: &str,
        amount: i64,
        balance: i64,
        status: &str,
        note: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<(), AccountError> {
        self.db.execute("INSERT INTO credit_transactions (id, user_id, actor_id, kind, amount_micro, balance_after_micro, status, note, created_at, completed_at, request_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", vec![Uuid::new_v4().to_string().into(), mysql::Value::from(user_id), mysql::Value::from(actor_id), mysql::Value::from(kind), mysql::Value::from(amount), mysql::Value::from(balance), mysql::Value::from(status), mysql::Value::from(note), mysql::Value::from(now()), mysql::Value::from(now()), mysql::Value::from(request_id)])?;
        Ok(())
    }
}

fn transaction_row(row: mysql::Row) -> TransactionView {
    let stream: i64 = row
        .get::<Option<i64>, _>(16)
        .unwrap_or_default()
        .unwrap_or_default();
    let http_status: Option<i64> = row.get::<Option<i64>, _>(20).unwrap_or_default();
    TransactionView {
        id: row
            .get::<Option<String>, _>(0)
            .unwrap_or_default()
            .unwrap_or_default(),
        user_id: row.get(1).unwrap_or_default(),
        username: row
            .get::<Option<String>, _>(2)
            .unwrap_or_default()
            .unwrap_or_default(),
        kind: row
            .get::<Option<String>, _>(3)
            .unwrap_or_default()
            .unwrap_or_default(),
        amount_micro: row.get(4).unwrap_or_default(),
        balance_after_micro: row.get(5).unwrap_or_default(),
        model: row.get::<Option<String>, _>(6).unwrap_or_default(),
        input_tokens: row.get(7).unwrap_or_default(),
        output_tokens: row.get(8).unwrap_or_default(),
        reserved_micro: row.get(9).unwrap_or_default(),
        status: row
            .get::<Option<String>, _>(10)
            .unwrap_or_default()
            .unwrap_or_default(),
        note: row.get::<Option<String>, _>(11).unwrap_or_default(),
        created_at: row
            .get::<Option<String>, _>(12)
            .unwrap_or_default()
            .unwrap_or_default(),
        completed_at: row.get::<Option<String>, _>(13).unwrap_or_default(),
        request_id: row.get::<Option<String>, _>(14).unwrap_or_default(),
        path: row.get::<Option<String>, _>(15).unwrap_or_default(),
        stream: stream != 0,
        balance_before_micro: row.get::<Option<i64>, _>(17).unwrap_or_default(),
        reserved_balance_micro: row.get::<Option<i64>, _>(18).unwrap_or_default(),
        refund_micro: row
            .get::<Option<i64>, _>(19)
            .unwrap_or_default()
            .unwrap_or_default(),
        http_status: http_status.map(|v| v as u16),
        error_message: row.get::<Option<String>, _>(21).unwrap_or_default(),
        duration_ms: row.get::<Option<i64>, _>(22).unwrap_or_default(),
        first_token_ms: row.get::<Option<i64>, _>(23).unwrap_or_default(),
    }
}

fn credits_to_micro(credits: f64) -> Result<i64, AccountError> {
    if !credits.is_finite() || credits.abs() > 1e12 {
        return Err(AccountError::InvalidInput("积分数量无效或超出限额".into()));
    }
    let micro = (credits * MICRO_CREDITS as f64).round();
    if micro > i64::MAX as f64 || micro < i64::MIN as f64 {
        return Err(AccountError::InvalidInput("积分数量超出限额".into()));
    }
    Ok(micro as i64)
}
pub fn key_hash(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}
fn key_prefix(key: &str) -> String {
    key.chars().take(12).collect()
}
fn generate_api_key() -> String {
    format!("sk-usr-{}", Uuid::new_v4().simple())
}
fn hash_password(password: &str) -> Result<String, AccountError> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|e| AccountError::Internal(format!("密码加密失败: {e}")))
}
fn verify_password(password: &str, hash: &str) -> Result<bool, AccountError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AccountError::Internal(format!("密码哈希损坏: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}
fn validate_password(password: &str) -> Result<(), AccountError> {
    if password.len() < 10 || password.len() > 256 {
        return Err(AccountError::InvalidInput("密码需为 10-256 个字符".into()));
    }
    Ok(())
}
fn now() -> String {
    Utc::now().to_rfc3339()
}
