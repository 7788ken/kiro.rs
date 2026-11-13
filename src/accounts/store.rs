use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::Utc;
use parking_lot::Mutex;
use rand_core::OsRng;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::models::{
    AccountOverview, AuthUser, BillingConfig, CreditChangeView, MICRO_CREDITS, RequestLog,
    Reservation, Role, TransactionView, UserView,
};

#[derive(Debug)]
pub enum AccountError {
    InvalidInput(String),
    NotFound,
    Conflict(String),
    Forbidden(String),
    InsufficientCredits {
        required_micro: i64,
        balance_micro: i64,
    },
    Internal(String),
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message)
            | Self::Conflict(message)
            | Self::Forbidden(message)
            | Self::Internal(message) => f.write_str(message),
            Self::NotFound => f.write_str("用户不存在"),
            Self::InsufficientCredits { .. } => f.write_str("积分余额不足"),
        }
    }
}

impl From<rusqlite::Error> for AccountError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Internal(format!("账户数据库错误: {error}"))
    }
}

pub struct AccountStore {
    connection: Mutex<Connection>,
}

impl AccountStore {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, AccountError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE COLLATE NOCASE,
                password_hash TEXT NOT NULL,
                api_key_hash TEXT NOT NULL UNIQUE,
                key_prefix TEXT NOT NULL,
                role TEXT NOT NULL CHECK (role IN ('super_admin', 'admin', 'user')),
                balance_micro INTEGER NOT NULL DEFAULT 0 CHECK (balance_micro >= 0),
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                parent_id INTEGER REFERENCES users(id)
            );
            CREATE TABLE IF NOT EXISTS billing_config (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                input_per_million REAL NOT NULL,
                output_per_million REAL NOT NULL
            );
            CREATE TABLE IF NOT EXISTS credit_transactions (
                id TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id),
                actor_id INTEGER REFERENCES users(id),
                kind TEXT NOT NULL,
                amount_micro INTEGER NOT NULL,
                balance_after_micro INTEGER NOT NULL,
                model TEXT,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                reserved_micro INTEGER NOT NULL DEFAULT 0,
                input_rate REAL NOT NULL DEFAULT 10.0,
                output_rate REAL NOT NULL DEFAULT 50.0,
                status TEXT NOT NULL,
                note TEXT,
                created_at TEXT NOT NULL,
                completed_at TEXT,
                request_id TEXT,
                path TEXT,
                stream INTEGER NOT NULL DEFAULT 0,
                balance_before_micro INTEGER,
                reserved_balance_micro INTEGER,
                http_status INTEGER,
                error_message TEXT,
                duration_ms INTEGER,
                first_token_ms INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_transactions_user ON credit_transactions(user_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_transactions_pending ON credit_transactions(status) WHERE status = 'pending';
            CREATE TABLE IF NOT EXISTS web_sessions (
                token_hash TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id),
                expires_at TEXT NOT NULL
            );",
        )?;
        let store = Self {
            connection: Mutex::new(connection),
        };
        let connection = store.connection.lock();
        ensure_column(
            &connection,
            "users",
            "parent_id",
            "INTEGER REFERENCES users(id)",
        )?;
        for (column, definition) in [
            ("request_id", "TEXT"),
            ("path", "TEXT"),
            ("stream", "INTEGER NOT NULL DEFAULT 0"),
            ("balance_before_micro", "INTEGER"),
            ("reserved_balance_micro", "INTEGER"),
            ("http_status", "INTEGER"),
            ("error_message", "TEXT"),
            ("duration_ms", "INTEGER"),
            ("first_token_ms", "INTEGER"),
        ] {
            ensure_column(&connection, "credit_transactions", column, definition)?;
        }
        connection.execute_batch(
            "UPDATE credit_transactions SET reserved_micro = -reserved_micro WHERE reserved_micro < 0;
             UPDATE credit_transactions SET error_message = '历史失败记录：旧版日志未保存具体错误信息' WHERE kind = 'charge' AND status = 'settled' AND amount_micro = 0 AND error_message IS NULL;
             CREATE INDEX IF NOT EXISTS idx_transactions_created ON credit_transactions(created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_transactions_request ON credit_transactions(request_id, created_at ASC);",
        )?;
        connection.execute(
            "INSERT OR IGNORE INTO billing_config VALUES (1, 10.0, 50.0)",
            [],
        )?;
        drop(connection);
        Ok(store)
    }

    pub fn ensure_super_admin(&self, admin_key: &str) -> Result<(), AccountError> {
        let connection = self.connection.lock();
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE role = 'super_admin')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            connection.execute(
                "INSERT INTO users (username, password_hash, api_key_hash, key_prefix, role, created_at)
                 VALUES ('superadmin', ?1, ?2, ?3, 'super_admin', ?4)",
                params![
                    hash_password(admin_key)?,
                    key_hash(admin_key),
                    key_prefix(admin_key),
                    now()
                ],
            )?;
        }
        Ok(())
    }

    pub fn super_admin(&self) -> Result<AuthUser, AccountError> {
        let connection = self.connection.lock();
        connection.query_row(
            "SELECT id, username, role, balance_micro, enabled, parent_id FROM users WHERE role = 'super_admin' ORDER BY id LIMIT 1",
            [],
            |row| Ok(AuthUser { id: row.get(0)?, username: row.get(1)?, role: Role::SuperAdmin, balance_micro: row.get(3)?, enabled: row.get::<_, i64>(4)? != 0, parent_id: row.get(5)? }),
        ).optional()?.filter(|user| user.enabled).ok_or_else(|| AccountError::Internal("超级管理员未初始化或已禁用".into()))
    }

    pub fn authenticate_key(&self, key: &str) -> Result<Option<AuthUser>, AccountError> {
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, username, role, balance_micro, enabled, parent_id FROM users WHERE api_key_hash = ?1",
        )?;
        let result = statement
            .query_row([key_hash(key)], |row| {
                Ok(AuthUser {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    role: Role::parse(&row.get::<_, String>(2)?).unwrap_or(Role::User),
                    balance_micro: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    parent_id: row.get(5)?,
                })
            })
            .optional()?;
        Ok(result.filter(|user| user.enabled))
    }

    pub fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<AuthUser>, AccountError> {
        let connection = self.connection.lock();
        let result = connection
            .query_row(
                "SELECT id, username, password_hash, role, balance_micro, enabled, parent_id FROM users WHERE username = ?1",
                [username.trim()],
                |row| {
                    Ok((
                        AuthUser {
                            id: row.get(0)?,
                            username: row.get(1)?,
                            role: Role::parse(&row.get::<_, String>(3)?).unwrap_or(Role::User),
                            balance_micro: row.get(4)?,
                            enabled: row.get::<_, i64>(5)? != 0,
                            parent_id: row.get(6)?,
                        },
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        match result {
            Some((user, hash))
                if user.enabled && verify_password(password, &hash).unwrap_or(false) =>
            {
                Ok(Some(user))
            }
            _ => Ok(None),
        }
    }

    pub fn create_web_session(&self, user: &AuthUser) -> Result<String, AccountError> {
        let token = Uuid::new_v4().simple().to_string() + &Uuid::new_v4().simple().to_string();
        let connection = self.connection.lock();
        connection.execute("DELETE FROM web_sessions WHERE expires_at <= ?1", [now()])?;
        connection.execute(
            "INSERT INTO web_sessions VALUES (?1, ?2, ?3)",
            params![
                key_hash(&token),
                user.id,
                (Utc::now() + chrono::Duration::hours(12)).to_rfc3339()
            ],
        )?;
        Ok(token)
    }

    pub fn authenticate_web_session(&self, token: &str) -> Result<Option<AuthUser>, AccountError> {
        let connection = self.connection.lock();
        let user_id = connection
            .query_row(
                "SELECT user_id FROM web_sessions WHERE token_hash = ?1 AND expires_at > ?2",
                params![key_hash(token), now()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match user_id {
            Some(id) => {
                let user = get_user(&connection, id)?;
                Ok(user.enabled.then_some(user))
            }
            None => Ok(None),
        }
    }

    pub fn revoke_web_session(&self, token: &str) -> Result<(), AccountError> {
        let connection = self.connection.lock();
        connection.execute(
            "DELETE FROM web_sessions WHERE token_hash = ?1",
            [key_hash(token)],
        )?;
        Ok(())
    }

    pub fn authenticate_session(&self, user_id: i64) -> Result<AuthUser, AccountError> {
        let connection = self.connection.lock();
        get_user(&connection, user_id).and_then(|user| {
            if user.enabled {
                Ok(user)
            } else {
                Err(AccountError::Forbidden("账户已被禁用".into()))
            }
        })
    }

    pub fn change_password(
        &self,
        user: &AuthUser,
        current_password: &str,
        new_password: &str,
    ) -> Result<String, AccountError> {
        validate_password(new_password)?;
        let connection = self.connection.lock();
        let current_hash: String = connection.query_row(
            "SELECT password_hash FROM users WHERE id = ?1",
            [user.id],
            |row| row.get(0),
        )?;
        if !verify_password(current_password, &current_hash)? {
            return Err(AccountError::Forbidden("当前密码不正确".into()));
        }
        let api_key = generate_api_key();
        connection.execute(
            "UPDATE users SET password_hash = ?1, api_key_hash = ?2, key_prefix = ?3 WHERE id = ?4",
            params![
                hash_password(new_password)?,
                key_hash(&api_key),
                key_prefix(&api_key),
                user.id
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
        let connection = self.connection.lock();
        let transaction = connection.unchecked_transaction()?;
        let result = transaction.execute(
            "INSERT INTO users (username, password_hash, api_key_hash, key_prefix, role, balance_micro, created_at, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![username, hash_password(password)?, key_hash(&api_key), key_prefix(&api_key), role.as_str(), amount, now(), actor.id],
        );
        if let Err(error) = result {
            return if error.to_string().contains("UNIQUE") {
                Err(AccountError::Conflict("用户名已存在".into()))
            } else {
                Err(error.into())
            };
        }
        let id = transaction.last_insert_rowid();
        if amount > 0 {
            insert_transaction(
                &transaction,
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
        transaction.commit()?;
        drop(connection);
        Ok((self.get_user_view(id)?, api_key))
    }

    pub fn list_users(&self, actor: &AuthUser) -> Result<Vec<UserView>, AccountError> {
        if !actor.is_manager() {
            return Err(AccountError::Forbidden("仅管理员可以查看用户列表".into()));
        }
        let connection = self.connection.lock();
        let mut statement = connection.prepare(
            "SELECT id, username, role, balance_micro, enabled, parent_id, key_prefix, created_at FROM users ORDER BY id",
        )?;
        let rows = statement.query_map([], user_view_row)?;
        let visible_ids = visible_user_ids(&connection, actor)?;
        let mut users = Vec::new();
        for row in rows {
            let mut user = row?;
            if visible_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&user.id))
            {
                continue;
            }
            add_usage(&connection, &mut user)?;
            users.push(user);
        }
        Ok(users)
    }

    pub fn get_user_view(&self, id: i64) -> Result<UserView, AccountError> {
        let connection = self.connection.lock();
        let mut user = connection.query_row(
            "SELECT id, username, role, balance_micro, enabled, parent_id, key_prefix, created_at FROM users WHERE id = ?1",
            [id],
            user_view_row,
        ).optional()?.ok_or(AccountError::NotFound)?;
        add_usage(&connection, &mut user)?;
        Ok(user)
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
        if !actor.role.can_manage(target.role)
            || role.is_some_and(|new_role| !actor.role.can_manage(new_role))
        {
            return Err(AccountError::Forbidden("无权修改该账户或角色".into()));
        }
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE users SET enabled = COALESCE(?1, enabled), role = COALESCE(?2, role) WHERE id = ?3",
            params![enabled.map(|v| v as i64), role.map(|r| r.as_str()), id],
        )?;
        drop(connection);
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
        let connection = self.connection.lock();
        connection.execute(
            "UPDATE users SET api_key_hash = ?1, key_prefix = ?2 WHERE id = ?3",
            params![key_hash(&api_key), key_prefix(&api_key), id],
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
        let connection = self.connection.lock();
        let transaction = connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE users SET balance_micro = balance_micro + ?1 WHERE id = ?2 AND balance_micro + ?1 >= 0",
            params![delta, id],
        )?;
        if changed == 0 {
            return Err(AccountError::InsufficientCredits {
                required_micro: -delta,
                balance_micro: target.balance_micro,
            });
        }
        let balance: i64 = transaction.query_row(
            "SELECT balance_micro FROM users WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        insert_transaction(
            &transaction,
            id,
            Some(actor.id),
            "adjustment",
            delta,
            balance,
            "settled",
            note,
            None,
        )?;
        transaction.commit()?;
        drop(connection);
        self.get_user_view(id)
    }

    pub fn get_billing_config(&self) -> Result<BillingConfig, AccountError> {
        let connection = self.connection.lock();
        Ok(connection.query_row(
            "SELECT input_per_million, output_per_million FROM billing_config WHERE id = 1",
            [],
            |row| {
                Ok(BillingConfig {
                    input_per_million: row.get(0)?,
                    output_per_million: row.get(1)?,
                })
            },
        )?)
    }

    pub fn set_billing_config(&self, config: &BillingConfig) -> Result<(), AccountError> {
        for price in [config.input_per_million, config.output_per_million] {
            if !price.is_finite() || !(0.0..=1_000_000_000.0).contains(&price) {
                return Err(AccountError::InvalidInput(
                    "单价必须在 0 至 1,000,000,000 积分/百万 token 之间".into(),
                ));
            }
        }
        let connection = self.connection.lock();
        connection.execute("UPDATE billing_config SET input_per_million = ?1, output_per_million = ?2 WHERE id = 1", params![config.input_per_million, config.output_per_million])?;
        Ok(())
    }

    pub fn overview(&self, user: &AuthUser) -> Result<AccountOverview, AccountError> {
        let user = self.authenticate_session(user.id)?;
        let connection = self.connection.lock();
        let (input, output, spent): (i64, i64, i64) = connection.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(-amount_micro), 0)
             FROM credit_transactions WHERE user_id = ?1 AND kind = 'charge' AND status = 'settled'",
            [user.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let pending_count = connection.query_row(
            "SELECT COUNT(*) FROM credit_transactions WHERE user_id = ?1 AND status = 'pending'",
            [user.id],
            |row| row.get(0),
        )?;
        drop(connection);
        Ok(AccountOverview {
            user,
            total_input_tokens: input,
            total_output_tokens: output,
            total_spent_micro: spent,
            pending_count,
            billing: self.get_billing_config()?,
        })
    }

    fn ensure_transaction_access(
        &self,
        actor: &AuthUser,
        requested_user: Option<i64>,
    ) -> Result<(), AccountError> {
        if actor.role == Role::SuperAdmin {
            return Ok(());
        }
        match requested_user {
            Some(user_id) if self.can_manage_account(actor, user_id)? => Ok(()),
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
            match requested_user {
                Some(_) => requested_user,
                None if actor.role == Role::SuperAdmin => None,
                None => Some(actor.id),
            }
        } else {
            Some(actor.id)
        };
        let connection = self.connection.lock();
        let sql = "SELECT t.id, t.user_id, u.username, t.kind, t.amount_micro, t.balance_after_micro, t.model,
                   t.input_tokens, t.output_tokens, t.reserved_micro, t.status, t.note, t.created_at, t.completed_at,
                   t.request_id, t.path, t.stream, t.balance_before_micro, t.reserved_balance_micro,
                   COALESCE(r.amount_micro, 0), t.http_status, t.error_message, t.duration_ms, t.first_token_ms
                   FROM credit_transactions t JOIN users u ON u.id = t.user_id
                   LEFT JOIN credit_transactions r ON r.request_id = t.request_id AND r.kind IN ('refund', 'release')
                   WHERE t.kind IN ('reserve', 'charge', 'error') ";
        let order = " ORDER BY t.created_at DESC, t.rowid DESC LIMIT ?1";
        let mut rows = Vec::new();
        if let Some(user_id) = user_id {
            let mut statement = connection.prepare(&format!("{sql}AND t.user_id = ?2{order}"))?;
            let results =
                statement.query_map(params![limit.min(500) as i64, user_id], transaction_row)?;
            for row in results {
                rows.push(row?);
            }
        } else {
            let mut statement = connection.prepare(&format!("{sql}{order}"))?;
            let results = statement.query_map([limit.min(500) as i64], transaction_row)?;
            for row in results {
                rows.push(row?);
            }
        }
        Ok(rows)
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
        let group_expression = match group_by {
            "day" => "strftime('%Y-%m-%d', COALESCE(t.completed_at, t.created_at), '+8 hours')",
            "user" => "u.username",
            "model" => "COALESCE(t.model, '未知模型')",
            "status" => {
                "CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN '失败' ELSE '成功' END"
            }
            _ => return Err(AccountError::InvalidInput("不支持的统计维度".into())),
        };
        let visible = visible_user_ids(&self.connection.lock(), actor)?;
        let scope = if let Some(ids) = visible {
            if let Some(user_id) = requested_user {
                format!(" AND t.user_id = {}", user_id)
            } else {
                format!(
                    " AND t.user_id IN ({})",
                    ids.into_iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        } else {
            String::new()
        };
        let filter = format!(
            "WHERE t.kind IN ('charge', 'error') AND t.status = 'settled' AND COALESCE(t.completed_at, t.created_at) >= ?1 AND COALESCE(t.completed_at, t.created_at) < ?2{scope}"
        );
        let connection = self.connection.lock();
        let mut statement = connection.prepare(&format!("SELECT {group_expression}, COUNT(*), SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 0 ELSE 1 END), SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 1 ELSE 0 END), SUM(t.input_tokens), SUM(t.output_tokens), SUM(t.input_tokens + t.output_tokens), SUM(MAX(-t.amount_micro, 0)), AVG(t.duration_ms), AVG(t.first_token_ms) FROM credit_transactions t JOIN users u ON u.id = t.user_id {filter} GROUP BY {group_expression} ORDER BY SUM(t.input_tokens + t.output_tokens) DESC, COUNT(*) DESC LIMIT ?3"))?;
        let groups = statement
            .query_map(params![start, end, limit.min(200)], |row| {
                Ok(crate::admin::usage_types::UsageGroup {
                    key: row.get(0)?,
                    requests: row.get(1)?,
                    successful_requests: row.get(2)?,
                    failed_requests: row.get(3)?,
                    input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    total_tokens: row.get(6)?,
                    spent_micro: row.get(7)?,
                    average_duration_ms: row
                        .get::<_, Option<f64>>(8)?
                        .map(|value| value.round() as i64),
                    average_first_token_ms: row
                        .get::<_, Option<f64>>(9)?
                        .map(|value| value.round() as i64),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let totals = connection.query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 0 ELSE 1 END), 0), COALESCE(SUM(CASE WHEN t.http_status >= 400 OR t.kind = 'error' THEN 1 ELSE 0 END), 0), COUNT(DISTINCT t.user_id), COALESCE(SUM(t.input_tokens), 0), COALESCE(SUM(t.output_tokens), 0), COALESCE(SUM(t.input_tokens + t.output_tokens), 0), COALESCE(SUM(MAX(-t.amount_micro, 0)), 0) FROM credit_transactions t JOIN users u ON u.id = t.user_id {filter}"),
            params![start, end],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?, row.get::<_, i64>(6)?, row.get::<_, i64>(7)?)),
        )?;
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
        let connection = self.connection.lock();
        let transaction = connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE users SET balance_micro = balance_micro - ?1 WHERE id = ?2 AND enabled = 1 AND balance_micro >= ?1",
            params![amount, user.id],
        )?;
        if changed == 0 {
            let balance = transaction
                .query_row(
                    "SELECT balance_micro FROM users WHERE id = ?1",
                    [user.id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(AccountError::NotFound)?;
            return Err(AccountError::InsufficientCredits {
                required_micro: amount,
                balance_micro: balance,
            });
        }
        let balance: i64 = transaction.query_row(
            "SELECT balance_micro FROM users WHERE id = ?1",
            [user.id],
            |row| row.get(0),
        )?;
        let balance_before = balance + amount;
        transaction.execute(
            "INSERT INTO credit_transactions (id, user_id, kind, amount_micro, balance_after_micro, model, input_tokens, output_tokens, reserved_micro, input_rate, output_rate, status, created_at, request_id, balance_before_micro, reserved_balance_micro)
             VALUES (?1, ?2, 'reserve', ?3, ?4, ?5, ?6, ?7, ?12, ?9, ?10, 'pending', ?8, ?1, ?11, ?4)",
            params![id, user.id, -amount, balance, model, estimated_input, output_limit, now(), billing.input_per_million, billing.output_per_million, balance_before, amount],
        )?;
        transaction.commit()?;
        Ok(Some(Reservation {
            id,
            user_id: user.id,
            amount_micro: amount,
            input_tokens: estimated_input,
        }))
    }

    pub fn settle(
        &self,
        reservation: &Reservation,
        success: bool,
        input_tokens: i64,
        output_tokens: i64,
        log: Option<&RequestLog>,
    ) -> Result<(), AccountError> {
        let connection = self.connection.lock();
        let transaction = connection.unchecked_transaction()?;
        let changed = transaction.execute("UPDATE credit_transactions SET status = 'processing' WHERE id = ?1 AND status = 'pending'", [&reservation.id])?;
        if changed == 0 {
            return Ok(());
        }
        let billing: BillingConfig = transaction.query_row(
            "SELECT input_rate, output_rate FROM credit_transactions WHERE id = ?1",
            [&reservation.id],
            |row| {
                Ok(BillingConfig {
                    input_per_million: row.get(0)?,
                    output_per_million: row.get(1)?,
                })
            },
        )?;
        let actual = if success {
            billing
                .cost_micro(input_tokens, output_tokens)
                .min(reservation.amount_micro)
        } else {
            0
        };
        let refund = reservation.amount_micro - actual;
        if refund > 0 {
            transaction.execute(
                "UPDATE users SET balance_micro = balance_micro + ?1 WHERE id = ?2",
                params![refund, reservation.user_id],
            )?;
        }
        let balance = transaction.query_row(
            "SELECT balance_micro FROM users WHERE id = ?1",
            [reservation.user_id],
            |row| row.get(0),
        )?;
        let reserved_balance_micro: Option<i64> = transaction.query_row(
            "SELECT reserved_balance_micro FROM credit_transactions WHERE id = ?1",
            [&reservation.id],
            |row| row.get(0),
        )?;
        let balance_before_micro = reserved_balance_micro
            .map(|balance_after_reservation| balance_after_reservation + reservation.amount_micro);
        transaction.execute(
            "UPDATE credit_transactions SET kind = 'charge', amount_micro = ?1, balance_after_micro = ?2, input_tokens = ?3, output_tokens = ?4, status = 'settled', completed_at = ?5, request_id = ?6, path = COALESCE(?7, path), model = COALESCE(?8, model), stream = COALESCE(?9, stream), balance_before_micro = COALESCE(?10, balance_before_micro), reserved_balance_micro = COALESCE(?11, reserved_balance_micro), http_status = ?12, error_message = ?13, duration_ms = ?14, first_token_ms = ?15 WHERE id = ?16",
            params![-actual, balance, input_tokens, output_tokens, now(), reservation.id, log.map(|log| log.path.as_str()), log.map(|log| log.model.as_str()), log.map(|log| log.stream as i64), balance_before_micro, reserved_balance_micro, log.map(|log| log.http_status), log.and_then(|log| log.error_message.as_deref()), log.map(|log| log.duration_ms), log.and_then(|log| log.first_token_ms), reservation.id],
        )?;
        insert_transaction(
            &transaction,
            reservation.user_id,
            None,
            if success { "refund" } else { "release" },
            refund,
            balance,
            "settled",
            Some(if success {
                "预授权未使用部分返还"
            } else {
                "请求失败，预授权全额返还"
            }),
            Some(&reservation.id),
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn refund(&self, reservation: &Reservation, log: Option<&RequestLog>) {
        if let Err(error) = self.settle(reservation, false, 0, 0, log) {
            tracing::error!(reservation = %reservation.id, "释放预授权积分失败: {}", error);
        }
    }

    pub fn can_manage_account(
        &self,
        actor: &AuthUser,
        target_id: i64,
    ) -> Result<bool, AccountError> {
        if actor.role == Role::SuperAdmin || actor.id == target_id {
            return Ok(true);
        }
        if !actor.is_manager() {
            return Ok(false);
        }
        self.is_descendant(actor.id, target_id)
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
            Some(owner_id) => self.can_manage_account(actor, owner_id),
            None => Ok(false),
        }
    }

    fn is_descendant(&self, ancestor_id: i64, target_id: i64) -> Result<bool, AccountError> {
        if ancestor_id == target_id {
            return Ok(true);
        }
        let connection = self.connection.lock();
        is_descendant(&connection, ancestor_id, target_id)
    }

    pub fn manageable_account_ids(
        &self,
        actor: &AuthUser,
    ) -> Result<std::collections::HashSet<i64>, AccountError> {
        let connection = self.connection.lock();
        visible_user_ids(&connection, actor).map(|ids| ids.unwrap_or_default())
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
        let result = self.connection.lock().execute(
            "INSERT INTO credit_transactions (id, user_id, kind, amount_micro, balance_after_micro, model, reserved_micro, status, created_at, completed_at, request_id, path, stream, http_status, error_message, duration_ms)
             VALUES (?1, ?2, 'error', 0, ?3, ?4, 0, 'settled', ?5, ?5, ?1, ?6, ?7, ?8, ?9, ?10)",
            params![format!("req_{}", Uuid::new_v4().simple()), user.id, user.balance_micro, model, now(), path, stream as i64, http_status, error_message, duration_ms],
        );
        if let Err(error) = result {
            tracing::error!("记录请求错误日志失败: {}", error);
        }
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
        let connection = self.connection.lock();
        let sql = "SELECT kind, amount_micro, balance_after_micro, note, created_at, completed_at
                   FROM credit_transactions WHERE request_id IS NULL AND kind IN ('initial', 'adjustment')";
        let order = " ORDER BY created_at DESC, rowid DESC LIMIT ?1";
        let mut changes = Vec::new();
        if let Some(user_id) = user_id {
            let mut statement = connection.prepare(&format!("{sql} AND user_id = ?2{order}"))?;
            let rows =
                statement.query_map(params![limit.min(500) as i64, user_id], credit_change_row)?;
            for row in rows {
                changes.push(row?);
            }
        } else {
            let mut statement = connection.prepare(&format!("{sql}{order}"))?;
            let rows = statement.query_map([limit.min(500) as i64], credit_change_row)?;
            for row in rows {
                changes.push(row?);
            }
        }
        Ok(changes)
    }
}

fn visible_user_ids(
    connection: &Connection,
    actor: &AuthUser,
) -> Result<Option<std::collections::HashSet<i64>>, AccountError> {
    if actor.role == Role::SuperAdmin {
        return Ok(None);
    }
    let mut statement = connection.prepare("SELECT id, parent_id FROM users")?;
    let relations = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    let mut children = std::collections::HashMap::<i64, Vec<i64>>::new();
    for relation in relations {
        let (id, parent_id) = relation?;
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
    Ok(Some(visible))
}

fn is_descendant(
    connection: &Connection,
    ancestor_id: i64,
    target_id: i64,
) -> Result<bool, AccountError> {
    let mut current_id = target_id;
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current_id) {
        let parent_id = connection
            .query_row(
                "SELECT parent_id FROM users WHERE id = ?1",
                [current_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        match parent_id {
            Some(id) if id == ancestor_id => return Ok(true),
            Some(id) => current_id = id,
            None => return Ok(false),
        }
    }
    Ok(false)
}

fn get_user(connection: &Connection, id: i64) -> Result<AuthUser, AccountError> {
    connection
        .query_row(
            "SELECT id, username, role, balance_micro, enabled, parent_id FROM users WHERE id = ?1",
            [id],
            |row| {
                Ok(AuthUser {
                    id: row.get(0)?,
                    username: row.get(1)?,
                    role: Role::parse(&row.get::<_, String>(2)?).unwrap_or(Role::User),
                    balance_micro: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    parent_id: row.get(5)?,
                })
            },
        )
        .optional()?
        .ok_or(AccountError::NotFound)
}

fn user_view_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserView> {
    Ok(UserView {
        id: row.get(0)?,
        username: row.get(1)?,
        role: Role::parse(&row.get::<_, String>(2)?).unwrap_or(Role::User),
        balance_micro: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        parent_id: row.get(5)?,
        key_prefix: row.get(6)?,
        created_at: row.get(7)?,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_spent_micro: 0,
    })
}

fn add_usage(connection: &Connection, user: &mut UserView) -> Result<(), AccountError> {
    let (input, output, spent): (i64, i64, i64) = connection.query_row(
        "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), COALESCE(SUM(-amount_micro), 0) FROM credit_transactions WHERE user_id = ?1 AND kind = 'charge' AND status = 'settled'",
        [user.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    user.total_input_tokens = input;
    user.total_output_tokens = output;
    user.total_spent_micro = spent;
    Ok(())
}

fn transaction_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransactionView> {
    Ok(TransactionView {
        id: row.get(0)?,
        user_id: row.get(1)?,
        username: row.get(2)?,
        kind: row.get(3)?,
        amount_micro: row.get(4)?,
        balance_after_micro: row.get(5)?,
        model: row.get(6)?,
        input_tokens: row.get(7)?,
        output_tokens: row.get(8)?,
        reserved_micro: row.get(9)?,
        status: row.get(10)?,
        note: row.get(11)?,
        created_at: row.get(12)?,
        completed_at: row.get(13)?,
        request_id: row.get(14)?,
        path: row.get(15)?,
        stream: row.get::<_, i64>(16)? != 0,
        balance_before_micro: row.get(17)?,
        reserved_balance_micro: row.get(18)?,
        refund_micro: row.get(19)?,
        http_status: row.get(20)?,
        error_message: row.get(21)?,
        duration_ms: row.get(22)?,
        first_token_ms: row.get(23)?,
    })
}

impl TransactionView {
    #[cfg(test)]
    fn is_legacy_failure(&self) -> bool {
        self.request_id.is_none()
            && self.reserved_micro > 0
            && self.amount_micro == 0
            && self.status == "settled"
    }
}

fn credit_change_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CreditChangeView> {
    Ok(CreditChangeView {
        kind: row.get(0)?,
        amount_micro: row.get(1)?,
        balance_after_micro: row.get(2)?,
        note: row.get(3)?,
        created_at: row.get(4)?,
        completed_at: row.get(5)?,
    })
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), AccountError> {
    let exists = {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<Result<Vec<_>, _>>()?
            .contains(&column.to_string())
    };
    if !exists {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn insert_transaction(
    connection: &Connection,
    user_id: i64,
    actor_id: Option<i64>,
    kind: &str,
    amount: i64,
    balance: i64,
    status: &str,
    note: Option<&str>,
    request_id: Option<&str>,
) -> Result<(), AccountError> {
    connection.execute(
        "INSERT INTO credit_transactions (id, user_id, actor_id, kind, amount_micro, balance_after_micro, status, note, created_at, completed_at, request_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![Uuid::new_v4().to_string(), user_id, actor_id, kind, amount, balance, status, note, now(), now(), request_id],
    )?;
    Ok(())
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
        .map(|hash| hash.to_string())
        .map_err(|error| AccountError::Internal(format!("密码加密失败: {error}")))
}

fn verify_password(password: &str, hash: &str) -> Result<bool, AccountError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|error| AccountError::Internal(format!("密码哈希损坏: {error}")))?;
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

#[cfg(all(test, not(feature = "mysql")))]
mod tests {
    use super::*;

    fn store() -> AccountStore {
        AccountStore::open(":memory:").unwrap()
    }

    fn manager(store: &AccountStore) -> AuthUser {
        store.ensure_super_admin("test-admin-key-123456").unwrap();
        store
            .authenticate_key("test-admin-key-123456")
            .unwrap()
            .unwrap()
    }

    #[test]
    fn billing_is_precise_and_nonzero() {
        let config = BillingConfig::default();
        assert_eq!(config.cost_micro(1, 1), 60);
        assert_eq!(config.cost_micro(1_000_000, 1_000_000), 60_000_000);
        assert_eq!(config.cost_micro(0, 0), 1);
    }

    #[test]
    fn reserve_and_settle_are_transactional_and_idempotent() {
        let store = store();
        let manager = manager(&store);
        let (_, key) = store
            .create_user(&manager, "alice", "password-123", Role::User, 10.0)
            .unwrap();
        let user = store.authenticate_key(&key).unwrap().unwrap();
        let reservation = store.reserve(&user, "sonnet", 1000, 1000).unwrap().unwrap();
        assert_eq!(reservation.amount_micro, 60_000);
        assert_eq!(
            store.authenticate_key(&key).unwrap().unwrap().balance_micro,
            9_940_000
        );
        store.settle(&reservation, true, 100, 100, None).unwrap();
        assert_eq!(
            store.authenticate_key(&key).unwrap().unwrap().balance_micro,
            9_994_000
        );
        store.settle(&reservation, true, 1000, 1000, None).unwrap();
        assert_eq!(
            store.authenticate_key(&key).unwrap().unwrap().balance_micro,
            9_994_000
        );
        let transactions = store.list_transactions(&user, None, 100).unwrap();
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].kind, "charge");
        assert_eq!(transactions[0].amount_micro, -6_000);
        assert_eq!(transactions[0].balance_before_micro, Some(10_000_000));
        assert_eq!(transactions[0].reserved_balance_micro, Some(9_940_000));
        assert_eq!(transactions[0].refund_micro, 54_000);
        assert_eq!(transactions[0].balance_after_micro, 9_994_000);
    }

    #[test]
    fn failed_request_releases_entire_reservation() {
        let store = store();
        let manager = manager(&store);
        let (_, key) = store
            .create_user(&manager, "alice", "password-123", Role::User, 1.0)
            .unwrap();
        let user = store.authenticate_key(&key).unwrap().unwrap();
        let reservation = store.reserve(&user, "sonnet", 100, 100).unwrap().unwrap();
        store.refund(&reservation, None);
        store.refund(&reservation, None);
        assert_eq!(
            store.authenticate_key(&key).unwrap().unwrap().balance_micro,
            MICRO_CREDITS
        );
        let transactions = store.list_transactions(&user, None, 100).unwrap();
        assert_eq!(transactions.len(), 1);
        assert!(transactions[0].is_legacy_failure() || transactions[0].refund_micro == 6_000);
        assert_eq!(transactions[0].amount_micro, 0);
        assert_eq!(transactions[0].balance_after_micro, MICRO_CREDITS);
    }

    #[test]
    fn insufficient_balance_does_not_create_reservation() {
        let store = store();
        let manager = manager(&store);
        let (_, key) = store
            .create_user(&manager, "alice", "password-123", Role::User, 0.0)
            .unwrap();
        let user = store.authenticate_key(&key).unwrap().unwrap();
        assert!(matches!(
            store.reserve(&user, "sonnet", 100, 100),
            Err(AccountError::InsufficientCredits { .. })
        ));
        assert_eq!(store.list_transactions(&user, None, 100).unwrap().len(), 0);
    }

    #[test]
    fn usage_stats_use_beijing_days_and_count_distinct_users() {
        let store = store();
        let manager = manager(&store);
        let (_, first_key) = store
            .create_user(&manager, "usage-first", "password-123", Role::User, 10.0)
            .unwrap();
        let (_, second_key) = store
            .create_user(&manager, "usage-second", "password-123", Role::User, 10.0)
            .unwrap();
        for (key, timestamp, model) in [
            (&first_key, "2026-11-12T15:59:59Z", "previous-day"),
            (&first_key, "2026-11-12T16:00:00Z", "sonnet"),
            (&second_key, "2026-11-13T15:59:59.999Z", "sonnet"),
        ] {
            let user = store.authenticate_key(key).unwrap().unwrap();
            let reservation = store.reserve(&user, model, 1000, 1000).unwrap().unwrap();
            store.settle(&reservation, true, 100, 100, None).unwrap();
            store
                .connection
                .lock()
                .execute(
                    "UPDATE credit_transactions SET completed_at = ?1 WHERE id = ?2",
                    params![timestamp, reservation.id],
                )
                .unwrap();
        }
        let stats = store
            .usage_stats(
                &manager,
                "2026-11-12T16:00:00+00:00",
                "2026-11-13T16:00:00+00:00",
                "day",
                None,
                100,
            )
            .unwrap();
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.active_users, 2);
        assert_eq!(stats.total_tokens, 400);
        assert_eq!(stats.spent_micro, 12_000);
        assert_eq!(stats.groups.len(), 1);
        assert_eq!(stats.groups[0].key, "2026-11-13");
        assert_eq!(stats.time_zone, "Asia/Shanghai");
    }

    #[test]
    fn role_and_credit_permissions_are_enforced() {
        let store = store();
        let super_admin = manager(&store);
        let (_, admin_key) = store
            .create_user(&super_admin, "manager", "password-123", Role::Admin, 0.0)
            .unwrap();
        let admin = store.authenticate_key(&admin_key).unwrap().unwrap();
        assert!(
            store
                .create_user(&admin, "other-admin", "password-123", Role::Admin, 0.0)
                .is_ok()
        );
        assert!(store.adjust_credits(&admin, admin.id, 100.0, None).is_err());
        assert!(
            store
                .update_user(&super_admin, super_admin.id, Some(false), None)
                .is_err()
        );
        let (_, user_key) = store
            .create_user(&admin, "alice", "password-123", Role::User, 5.0)
            .unwrap();
        let user = store.authenticate_key(&user_key).unwrap().unwrap();
        assert_eq!(user.balance_micro, 5_000_000);
        assert!(store.list_users(&user).is_err());
    }

    #[test]
    fn password_login_and_key_rotation_invalidate_old_key() {
        let store = store();
        let manager = manager(&store);
        let (_, key) = store
            .create_user(&manager, "alice", "password-123", Role::User, 1.0)
            .unwrap();
        let user = store
            .authenticate_password("alice", "password-123")
            .unwrap()
            .unwrap();
        assert!(
            store
                .authenticate_password("alice", "incorrect")
                .unwrap()
                .is_none()
        );
        let new_key = store
            .change_password(&user, "password-123", "new-password-123")
            .unwrap();
        assert!(store.authenticate_key(&key).unwrap().is_none());
        assert!(store.authenticate_key(&new_key).unwrap().is_some());
        assert!(
            store
                .authenticate_password("alice", "new-password-123")
                .unwrap()
                .is_some()
        );
    }
}
