//! Admin API 类型定义

use serde::{Deserialize, Serialize};

// ============ 凭据状态 ============

/// 所有凭据状态响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatusResponse {
    /// 凭据总数
    pub total: usize,
    /// 可用凭据数量（未禁用）
    pub available: usize,
    /// 当前活跃凭据 ID
    pub current_id: u64,
    /// 各凭据状态列表
    pub credentials: Vec<CredentialStatusItem>,
}

/// 单个凭据的状态信息
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatusItem {
    /// 凭据唯一 ID
    pub id: u64,
    /// 优先级（数字越小优先级越高）
    pub priority: u32,
    /// 是否被禁用
    pub disabled: bool,
    /// 连续失败次数
    pub failure_count: u32,
    /// 是否为当前活跃凭据
    pub is_current: bool,
    /// Token 过期时间（RFC3339 格式）
    pub expires_at: Option<String>,
    /// 认证方式
    pub auth_method: Option<String>,
    /// 是否有 Profile ARN
    pub has_profile_arn: bool,
    /// refreshToken 的 SHA-256 哈希（仅 OAuth 凭据，用于前端去重）
    pub refresh_token_hash: Option<String>,
    /// kiroApiKey 的 SHA-256 哈希（仅 API Key 凭据，用于前端去重）
    pub api_key_hash: Option<String>,
    /// kiroApiKey 的脱敏展示（仅 API Key 凭据，用于前端显示）
    pub masked_api_key: Option<String>,
    /// 用户邮箱（用于前端显示）
    pub email: Option<String>,
    /// API 调用成功次数
    pub success_count: u64,
    /// 最后一次 API 调用时间（RFC3339 格式）
    pub last_used_at: Option<String>,
    /// 累计消耗的上游积分（metering 事件 credits，不含用户结算倍率）
    pub used_credits: f64,
    /// 是否配置了凭据级代理
    pub has_proxy: bool,
    /// 代理 URL（用于前端展示；绑定代理池时为池条目地址，不含密码）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// 绑定的代理池条目 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_id: Option<u64>,
    /// 绑定的代理池条目名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_name: Option<String>,
    /// 绑定代理最近一次检测到的出口 IP
    #[serde(skip_serializing_if = "Option::is_none")]
    pub egress_ip: Option<String>,
    /// Token 刷新连续失败次数
    pub refresh_failure_count: u32,
    /// 登记该凭据的管理员账户 ID
    pub registered_by: Option<i64>,
    /// 登记时的管理员名称
    pub registered_by_name: Option<String>,
    /// 当前登录管理员是否可管理该凭据
    pub can_manage: bool,
    /// 禁用原因
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    /// 端点名称（决定该凭据走哪套 Kiro API，已回退到默认端点）
    pub endpoint: String,
    /// 是否已归档
    pub archived: bool,
    /// 归档时间（RFC3339）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    /// 执行归档的管理员账户 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_by: Option<i64>,
    /// 归档原因
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_reason: Option<String>,
}

// ============ 操作请求 ============

/// 归档凭据请求
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCredentialRequest {
    /// 归档原因（可选）
    #[serde(default)]
    pub reason: Option<String>,
}

/// 启用/禁用凭据请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDisabledRequest {
    /// 是否禁用
    pub disabled: bool,
}

/// 修改优先级请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPriorityRequest {
    /// 新优先级值
    pub priority: u32,
}

/// 添加凭据请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCredentialRequest {
    /// 刷新令牌（OAuth 凭据必填，API Key 凭据不需要）
    pub refresh_token: Option<String>,

    /// 认证方式（可选，默认 social）
    #[serde(default = "default_auth_method")]
    pub auth_method: String,

    /// OIDC Client ID（IdC 认证需要）
    pub client_id: Option<String>,

    /// OIDC Client Secret（IdC 认证需要）
    pub client_secret: Option<String>,

    /// 优先级（可选，默认 0）
    #[serde(default)]
    pub priority: u32,

    /// AWS Profile ARN（企业 IdC 的额度和 API 请求使用）
    pub profile_arn: Option<String>,

    /// 凭据级 Region 配置（用于 OIDC token 刷新）
    /// 未配置时回退到 config.json 的全局 region
    pub region: Option<String>,

    /// 凭据级 Auth Region（用于 Token 刷新）
    pub auth_region: Option<String>,

    /// 凭据级 API Region（用于 API 请求）
    pub api_region: Option<String>,

    /// 凭据级 Machine ID（可选，64 位字符串）
    /// 未配置时回退到 config.json 的 machineId
    pub machine_id: Option<String>,

    /// 用户邮箱（可选，用于前端显示）
    pub email: Option<String>,

    /// 凭据级代理 URL（可选，特殊值 "direct" 表示不使用代理）
    pub proxy_url: Option<String>,

    /// 凭据级代理认证用户名（可选）
    pub proxy_username: Option<String>,

    /// 凭据级代理认证密码（可选）
    pub proxy_password: Option<String>,

    /// 绑定的代理池条目 ID（可选，与 proxyUrl 互斥；都不传时自动从健康代理中分配）
    #[serde(default)]
    pub proxy_id: Option<u64>,

    /// Kiro API Key（API Key 凭据必填，格式: ksk_xxxxxxxx）
    /// 设置后直接作为 Bearer Token 使用，无需 refreshToken
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kiro_api_key: Option<String>,

    /// 端点名称（可选，未配置时使用 config.defaultEndpoint）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

fn default_auth_method() -> String {
    "social".to_string()
}

/// 添加凭据成功响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCredentialResponse {
    pub success: bool,
    pub message: String,
    /// 新添加的凭据 ID
    pub credential_id: u64,
    /// 用户邮箱（如果获取成功）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// 修改凭据代理绑定请求（三选一）
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCredentialProxyRequest {
    /// 绑定代理池条目
    #[serde(default)]
    pub proxy_id: Option<u64>,
    /// 设为直连
    #[serde(default)]
    pub direct: bool,
    /// 清空，回退到全局代理
    #[serde(default)]
    pub clear: bool,
}

// ============ 出口代理池 ============

/// 代理池条目（对外展示，不含密码）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyItem {
    pub id: u64,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 是否已配置密码（密码本身不返回）
    pub password_set: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub health: crate::proxy_pool::ProxyHealth,
    /// 绑定该代理的未归档凭据数
    pub bound_count: usize,
}

/// 代理池列表响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyListResponse {
    pub proxies: Vec<ProxyItem>,
}

/// 批量导入代理请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProxiesRequest {
    /// 每行一个代理，空行和 `#` 注释行忽略
    pub text: String,
    /// 行中没有协议前缀时使用，默认 socks5
    #[serde(default)]
    pub default_scheme: Option<String>,
    /// 导入后立即检测新增代理，默认 true
    #[serde(default)]
    pub check_after_import: Option<bool>,
}

/// 批量导入代理响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProxiesResponse {
    /// 新增的代理（不含密码，health 为导入后检测结果或未知）
    pub added: Vec<ProxyItem>,
    pub skipped: Vec<crate::proxy_pool::BatchLineIssue>,
    pub failed: Vec<crate::proxy_pool::BatchLineIssue>,
    /// 原文总行数（含空行和注释）
    pub total_lines: usize,
}

/// 设置代理启用状态请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProxyEnabledRequest {
    pub enabled: bool,
}

/// 凭据出口检测响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialEgressResponse {
    pub credential_id: u64,
    /// 实际使用的代理地址（不含密码），直连时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    pub health: crate::proxy_pool::ProxyHealth,
}

// ============ 余额查询 ============

/// 余额查询响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceResponse {
    /// 凭据 ID
    pub id: u64,
    /// 订阅类型
    pub subscription_title: Option<String>,
    /// 当前使用量
    pub current_usage: f64,
    /// 使用限额
    pub usage_limit: f64,
    /// 剩余额度
    pub remaining: f64,
    /// 使用百分比
    pub usage_percentage: f64,
    /// 下次重置时间（Unix 时间戳）
    pub next_reset_at: Option<f64>,
}

// ============ 负载均衡配置 ============

/// 负载均衡模式响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancingModeResponse {
    /// 当前模式（"priority" 或 "balanced"）
    pub mode: String,
}

/// 设置负载均衡模式请求
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLoadBalancingModeRequest {
    /// 模式（"priority" 或 "balanced"）
    pub mode: String,
}

// ============ 通用响应 ============

/// 操作成功响应
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

impl SuccessResponse {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
        }
    }
}

/// 错误响应
#[derive(Debug, Serialize)]
pub struct AdminErrorResponse {
    pub error: AdminError,
}

#[derive(Debug, Serialize)]
pub struct AdminError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

impl AdminErrorResponse {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: AdminError {
                error_type: error_type.into(),
                message: message.into(),
            },
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }

    pub fn authentication_error() -> Self {
        Self::new("authentication_error", "Invalid or missing admin API key")
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub fn api_error(message: impl Into<String>) -> Self {
        Self::new("api_error", message)
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new("internal_error", message)
    }
}
