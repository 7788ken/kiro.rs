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

const MANAGER_CHAT_FORBIDDEN: &str =
    "管理员账户的 API Key 不能调用对话接口，请使用普通用户的 Key";

/// 是否为对话接口（`/v1/messages`、`/cc/v1/messages`，不含 `count_tokens`）
fn is_chat_path(path: &str) -> bool {
    matches!(path, "/v1/messages" | "/cc/v1/messages" | "/messages")
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
                let path = request
                    .extensions()
                    .get::<axum::extract::OriginalUri>()
                    .map(|uri| uri.0.path().to_string())
                    .unwrap_or_else(|| request.uri().path().to_string());
                let live = accounts.tracker.begin(&user, &path, "", false);
                // 管理员账户不计费，因此不允许调用对话接口（含 config.json 的 apiKey / adminApiKey）
                if is_chat_path(&path) {
                    live.finish(
                        StatusCode::FORBIDDEN.as_u16(),
                        0,
                        0,
                        None,
                        Some(MANAGER_CHAT_FORBIDDEN.to_string()),
                    );
                    return (
                        StatusCode::FORBIDDEN,
                        Json(ErrorResponse::permission_error(MANAGER_CHAT_FORBIDDEN)),
                    )
                        .into_response();
                }
                // 注入用户信息，下游 billing_middleware 依赖它判断管理员直通
                let mut request = request;
                request.extensions_mut().insert(user);
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

#[cfg(all(test, not(feature = "mysql")))]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::json;
    use tower::ServiceExt;

    use super::MANAGER_CHAT_FORBIDDEN;
    use crate::accounts::{AccountState, AccountStore, Role};

    const ADMIN_KEY: &str = "test-super-admin-key";
    const LEGACY_API_KEY: &str = "test-legacy-api-key";

    fn fixture() -> (Router, String) {
        let store = AccountStore::open(":memory:").unwrap();
        store.ensure_super_admin(ADMIN_KEY).unwrap();
        let admin = store.authenticate_key(ADMIN_KEY).unwrap().unwrap();
        let (_, user_key) = store
            .create_user(&admin, "chat-user", "password-123", Role::User, 1.0)
            .unwrap();
        let accounts =
            AccountState::new(Arc::new(store), ADMIN_KEY).with_legacy_api_key(LEGACY_API_KEY);
        (
            crate::anthropic::create_router_with_accounts(None, false, accounts),
            user_key,
        )
    }

    async fn send(app: &Router, method: &str, uri: &str, key: &str) -> (StatusCode, String) {
        let body = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 16,
            "messages": [{"role": "user", "content": "hello"}]
        });
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("x-api-key", key)
            .header("content-type", "application/json");
        let request = if method == "GET" {
            request.body(Body::empty())
        } else {
            request.body(Body::from(body.to_string()))
        }
        .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn manager_keys_cannot_call_chat_endpoints() {
        let (app, _) = fixture();
        for key in [ADMIN_KEY, LEGACY_API_KEY] {
            for uri in ["/v1/messages", "/cc/v1/messages"] {
                let (status, body) = send(&app, "POST", uri, key).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{uri} {body}");
                assert!(body.contains(MANAGER_CHAT_FORBIDDEN), "{body}");
                assert!(body.contains("permission_error"), "{body}");
            }
        }
    }

    #[tokio::test]
    async fn manager_keys_can_still_list_models_and_count_tokens() {
        let (app, _) = fixture();
        let (status, body) = send(&app, "GET", "/v1/models", ADMIN_KEY).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // /v1/messages/count_tokens 挂在 billing_middleware 下，管理员分支已注入 AuthUser，
        // billing_middleware 对管理员直通，应正常返回 200
        let (status, body) = send(&app, "POST", "/v1/messages/count_tokens", ADMIN_KEY).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(!body.contains(MANAGER_CHAT_FORBIDDEN), "{body}");
        let (status, body) = send(&app, "POST", "/cc/v1/messages/count_tokens", ADMIN_KEY).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn user_keys_reach_chat_handler() {
        let (app, user_key) = fixture();
        // 测试环境没有 KiroProvider，走到原有 handler 后返回 503，而不是 403
        let (status, body) = send(&app, "POST", "/v1/messages", &user_key).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        assert!(!body.contains(MANAGER_CHAT_FORBIDDEN), "{body}");
    }
}
