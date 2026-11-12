//! Anthropic API 路由配置

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};

use crate::{
    accounts::{AccountState, billing::billing_middleware},
    kiro::provider::KiroProvider,
};

use super::{
    handlers::{count_tokens, get_models, post_messages, post_messages_cc},
    middleware::{AppState, auth_middleware, cors_layer},
};

/// 请求体最大大小限制 (50MB)
const MAX_BODY_SIZE: usize = 50 * 1024 * 1024;

/// 创建带有用户鉴权和积分计费的 Anthropic API 路由
pub fn create_router_with_accounts(
    kiro_provider: Option<KiroProvider>,
    extract_thinking: bool,
    accounts: AccountState,
) -> Router {
    let mut state = AppState::new("", extract_thinking).with_accounts(accounts.clone());
    if let Some(provider) = kiro_provider {
        state = state.with_kiro_provider(provider);
    }

    let billed_messages = Router::new()
        .route("/messages", post(post_messages))
        .route("/messages/count_tokens", post(count_tokens))
        .layer(middleware::from_fn_with_state(
            accounts.clone(),
            billing_middleware,
        ));

    let v1_routes = Router::new()
        .route("/models", get(get_models))
        .merge(billed_messages)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let cc_messages = Router::new()
        .route("/messages", post(post_messages_cc))
        .layer(middleware::from_fn_with_state(accounts, billing_middleware));
    let cc_v1_routes = Router::new()
        .merge(cc_messages)
        .route("/messages/count_tokens", post(count_tokens))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .nest("/v1", v1_routes)
        .nest("/cc/v1", cc_v1_routes)
        .layer(cors_layer())
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .with_state(state)
}
