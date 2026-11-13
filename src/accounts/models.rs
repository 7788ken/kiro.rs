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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionQuery {
    pub user_id: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingConfig {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

impl BillingConfig {
    pub fn cost_micro(&self, input_tokens: i64, output_tokens: i64) -> i64 {
        let input = (input_tokens.max(0) as f64 * self.input_per_million).ceil() as i64;
        let output = (output_tokens.max(0) as f64 * self.output_per_million).ceil() as i64;
        input.saturating_add(output).max(1)
    }
}

impl Default for BillingConfig {
    fn default() -> Self {
        Self {
            input_per_million: 10.0,
            output_per_million: 50.0,
        }
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
