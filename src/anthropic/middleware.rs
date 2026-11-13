//! Anthropic API 中间件

use parking_lot::RwLock;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

use crate::kiro::provider::KiroProvider;
use crate::{accounts::AccountState, common::auth};

use super::types::ErrorResponse;

/// 应用共享状态
#[derive(Clone)]
pub struct AppState {
    /// API 密钥
    pub api_key: String,
    /// Kiro Provider（可选，用于实际 API 调用）
    /// 内部使用 MultiTokenManager，已支持线程安全的多凭据管理
    pub kiro_provider: Option<Arc<RwLock<Arc<KiroProvider>>>>,
    /// 是否开启非流式响应的 thinking 块提取
    pub extract_thinking: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 账户服务，用于用户鉴权和积分计费
    pub accounts: Option<AccountState>,
    pub tracker: crate::admin::RequestTracker,
}

impl AppState {
    /// 创建新的应用状态
    pub fn new(api_key: impl Into<String>, extract_thinking: bool) -> Self {
        Self {
            api_key: api_key.into(),
            kiro_provider: None,
            extract_thinking: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                extract_thinking,
            )),
            accounts: None,
            tracker: crate::admin::RequestTracker::new(),
        }
    }

    /// 设置 KiroProvider
    pub fn with_kiro_provider(mut self, provider: KiroProvider) -> Self {
        self.kiro_provider = Some(Arc::new(RwLock::new(Arc::new(provider))));
        self
    }

    pub fn with_accounts(mut self, accounts: AccountState) -> Self {
        self.tracker = accounts.tracker.clone();
        self.accounts = Some(accounts);
        self
    }
}

/// API Key 认证中间件
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Some(accounts) = state.accounts.clone() {
        let Some(key) = auth::extract_api_key(&request) else {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse::authentication_error()),
            )
                .into_response();
        };
        match crate::accounts::middleware::authenticate_api_key(&accounts, &key).await {
            Ok(user) if user.is_manager() => {
                let path = request.uri().path().to_string();
                let live = accounts.tracker.begin(&user, &path, "", false);
                let response = next.run(request).await;
                live.finish(response.status().as_u16(), 0, 0, None, None);
                response
            }
            Ok(user) => {
                let mut request = request;
                request.extensions_mut().insert(user);
                next.run(request).await
            }
            Err(response) => response,
        }
    } else {
        match auth::extract_api_key(&request) {
            Some(key) if auth::constant_time_eq(&key, &state.api_key) => next.run(request).await,
            _ => {
                let error = ErrorResponse::authentication_error();
                (StatusCode::UNAUTHORIZED, Json(error)).into_response()
            }
        }
    }
}

/// CORS 中间件层
///
/// **安全说明**：当前配置允许所有来源（Any），这是为了支持公开 API 服务。
/// 如果需要更严格的安全控制，请根据实际需求配置具体的允许来源、方法和头信息。
///
/// # 配置说明
/// - `allow_origin(Any)`: 允许任何来源的请求
/// - `allow_methods(Any)`: 允许任何 HTTP 方法
/// - `allow_headers(Any)`: 允许任何请求头
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{Any, CorsLayer};

    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
}
