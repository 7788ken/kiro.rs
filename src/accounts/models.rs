use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub const MICRO_CREDITS: i64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    SuperAdmin,
    Admin,
    User,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SuperAdmin => "super_admin",
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "super_admin" => Some(Self::SuperAdmin),
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User),
            _ => None,
        }
    }

    pub fn can_create(self, target: Self) -> bool {
        match self {
            Self::SuperAdmin => target != Self::SuperAdmin,
            Self::Admin => target != Self::SuperAdmin,
            Self::User => false,
        }
    }

    pub fn can_manage(self, target: Self) -> bool {
        match self {
            Self::SuperAdmin => target != Self::SuperAdmin,
            Self::Admin => target != Self::SuperAdmin,
            Self::User => false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub parent_id: Option<i64>,
    pub balance_micro: i64,
    pub enabled: bool,
}

impl AuthUser {
    pub fn is_manager(&self) -> bool {
        self.role != Role::User
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserView {
    pub id: i64,
    pub username: String,
    pub role: Role,
    pub parent_id: Option<i64>,
    pub balance_micro: i64,
    pub enabled: bool,
    pub key_prefix: String,
    pub created_at: String,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_spent_micro: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default = "default_user_role")]
    pub role: Role,
    #[serde(default)]
    pub initial_balance: f64,
}

fn default_user_role() -> Role {
    Role::User
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserResponse {
    pub user: UserView,
    pub api_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    pub enabled: Option<bool>,
    pub role: Option<Role>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustCreditsRequest {
    pub amount: f64,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionView {
    pub id: String,
    pub user_id: i64,
    pub username: String,
    pub kind: String,
    pub amount_micro: i64,
    pub balance_after_micro: i64,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reserved_micro: i64,
    pub status: String,
    pub note: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub request_id: Option<String>,
    pub path: Option<String>,
    pub stream: bool,
    pub balance_before_micro: Option<i64>,
    pub reserved_balance_micro: Option<i64>,
    pub refund_micro: i64,
    pub http_status: Option<u16>,
    pub error_message: Option<String>,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    /// 本次请求使用的上游凭证 ID（旧记录为空）
    pub credential_id: Option<u64>,
    /// 模拟的 prompt cache 写入 token（已包含在 input_tokens 内，旧记录为 0）
    pub cache_creation_input_tokens: i64,
    /// 模拟的 prompt cache 读取 token（已包含在 input_tokens 内，旧记录为 0）
    pub cache_read_input_tokens: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditChangeView {
    pub kind: String,
    pub amount_micro: i64,
    pub balance_after_micro: i64,
    pub note: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RequestLog {
    pub path: String,
    pub model: String,
    pub stream: bool,
    pub http_status: u16,
    pub error_message: Option<String>,
    pub duration_ms: i64,
    pub first_token_ms: Option<i64>,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionQuery {
    pub user_id: Option<i64>,
    pub limit: Option<u32>,
}

fn default_consumption_ratio() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingConfig {
    pub input_per_million: f64,
    pub output_per_million: f64,
    /// 用户积分 = 凭证本次真实消耗 × 倍率。1 为 1:1。
    #[serde(default = "default_consumption_ratio")]
    pub consumption_ratio: f64,
}

impl BillingConfig {
    pub fn cost_micro(&self, input_tokens: i64, output_tokens: i64) -> i64 {
        let input = (input_tokens.max(0) as f64 * self.input_per_million).ceil() as i64;
        let output = (output_tokens.max(0) as f64 * self.output_per_million).ceil() as i64;
        input.saturating_add(output).max(1)
    }

    /// 预扣也乘以消费倍率，避免高倍率请求按原价冻结。
    pub fn hold_micro(&self, input_tokens: i64, output_tokens: i64) -> i64 {
        scale_micro(self.cost_micro(input_tokens, output_tokens) as f64, self.consumption_ratio)
            .max(1)
    }

    pub fn validate(&self) -> Result<(), String> {
        for price in [self.input_per_million, self.output_per_million] {
            if !price.is_finite() || !(0.0..=1_000_000_000.0).contains(&price) {
                return Err(
                    "单价必须在 0 至 1,000,000,000 积分/百万 token 之间".into(),
                );
            }
        }
        if !self.consumption_ratio.is_finite()
            || self.consumption_ratio <= 0.0
            || self.consumption_ratio > 1_000.0
        {
            return Err("消费倍率必须大于 0，且不超过 1000".into());
        }
        Ok(())
    }
}

impl Default for BillingConfig {
    fn default() -> Self {
        Self {
            input_per_million: 10.0,
            output_per_million: 50.0,
            consumption_ratio: default_consumption_ratio(),
        }
    }
}

const MAX_CHARGE_MICRO: i64 = 1_000_000_000_000_000;

fn normalize_ratio(ratio: f64) -> f64 {
    if ratio.is_finite() && ratio > 0.0 {
        ratio
    } else {
        1.0
    }
}

fn scale_micro(amount: f64, ratio: f64) -> i64 {
    if !amount.is_finite() || amount <= 0.0 {
        return 0;
    }
    let micro = (amount * normalize_ratio(ratio)).round();
    if micro >= MAX_CHARGE_MICRO as f64 {
        MAX_CHARGE_MICRO
    } else {
        micro as i64
    }
}

/// 凭证真实消耗乘以结算倍率，得到应向用户收取的积分。
///
/// 没有报告、非有限值或负数都按 0 处理，不再回退到 Token 单价。
pub fn credential_charge_micro(credits: Option<f64>, ratio: f64) -> i64 {
    let Some(credits) = credits else {
        return 0;
    };
    if !credits.is_finite() || credits <= 0.0 {
        return 0;
    }
    scale_micro(credits * MICRO_CREDITS as f64, ratio)
}

fn format_ratio(ratio: f64) -> String {
    let text = format!("{:.4}", normalize_ratio(ratio));
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn is_unit_ratio(ratio: f64) -> bool {
    (normalize_ratio(ratio) - 1.0).abs() < 1e-9
}

/// 一次结算要写进流水的说明。
///
/// `movement_micro` 为正是退回预扣，为负是从剩余余额补扣。
pub fn settlement_notes(
    success: bool,
    credits: Option<f64>,
    reported_micro: i64,
    actual_micro: i64,
    movement_micro: i64,
    ratio: f64,
) -> (String, &'static str, String) {
    let due = reported_micro as f64 / MICRO_CREDITS as f64;
    let charge_note = if reported_micro > 0 && actual_micro < reported_micro {
        match credits {
            Some(credits) if !is_unit_ratio(ratio) => format!(
                "凭证消耗 {credits:.6}，按 {} 倍率应收 {due:.6}，可用余额不足，已扣至 0",
                format_ratio(ratio)
            ),
            Some(credits) => {
                format!("凭证实际消耗 {credits:.6} 积分，可用余额不足，已扣至 0")
            }
            None => "凭证实际消耗超过可用余额，已扣至 0".to_string(),
        }
    } else if let Some(credits) = credits.filter(|value| value.is_finite()) {
        if is_unit_ratio(ratio) {
            format!("按凭证实际消耗 {credits:.6} 积分结算")
        } else {
            format!(
                "凭证消耗 {credits:.6}，按 {} 倍率收取 {due:.6} 积分",
                format_ratio(ratio)
            )
        }
    } else if success {
        "上游未返回凭证消耗，本次不扣积分".to_string()
    } else {
        "请求失败，预授权全额返还".to_string()
    };
    let (kind, adjustment_note) = if movement_micro < 0 {
        (
            "surcharge",
            "凭证消耗超过预扣，已从剩余余额补扣".to_string(),
        )
    } else if movement_micro == 0 {
        ("refund", "预扣与凭证消耗一致".to_string())
    } else if actual_micro == 0 && !success {
        ("release", "请求失败，预授权全额返还".to_string())
    } else if actual_micro == 0 && credits.is_none() {
        (
            "refund",
            "上游未返回凭证消耗，预授权已全额返还".to_string(),
        )
    } else if actual_micro == 0 {
        ("refund", "凭证实际消耗为 0，预授权已全额返还".to_string())
    } else {
        ("refund", "预授权未使用部分返还".to_string())
    };
    (charge_note, kind, adjustment_note)
}

/// 单次请求从上游 metering 事件收集到的凭证消耗。
///
/// 计费中间件创建一份，请求处理过程中写入，结算时读取。
#[derive(Clone, Debug, Default)]
pub struct CredentialUsage {
    credits: Arc<Mutex<Option<f64>>>,
    /// 本次请求最终成功响应所用的凭证 ID（重试换凭证时以最后成功的为准）
    credential_id: Arc<Mutex<Option<u64>>>,
}

impl CredentialUsage {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录本次请求的凭证消耗。后到的事件覆盖先前的值。
    pub fn record(&self, credits: f64) {
        if !credits.is_finite() || credits < 0.0 {
            return;
        }
        *self.credits.lock() = Some(credits);
    }

    pub fn get(&self) -> Option<f64> {
        *self.credits.lock()
    }

    /// 记录本次请求使用的凭证 ID。后写入的覆盖先前的值。
    pub fn set_credential_id(&self, id: u64) {
        *self.credential_id.lock() = Some(id);
    }

    pub fn credential_id(&self) -> Option<u64> {
        *self.credential_id.lock()
    }
}

#[derive(Debug, Clone)]
pub struct Reservation {
    pub id: String,
    pub user_id: i64,
    pub amount_micro: i64,
    pub input_tokens: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountOverview {
    pub user: AuthUser,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_spent_micro: i64,
    pub pending_count: i64,
    pub billing: BillingConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}
