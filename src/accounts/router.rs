use super::models::AuthUser;
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;

use super::{
    middleware::{AccountState, account_auth_middleware, admin_auth_middleware, error_response},
    models::{
        AdjustCreditsRequest, BillingConfig, ChangePasswordRequest, CreateUserRequest,
        CreateUserResponse, TransactionFacetsQuery, TransactionQuery, UpdateUserRequest,
    },
    store::AccountError,
};
use crate::admin::types::{AdminErrorResponse, SuccessResponse};

pub fn account_router(state: AccountState) -> Router {
    Router::new()
        .route("/me", get(get_me))
        .route("/password", post(change_password))
        .route("/key", post(regenerate_own_key))
        .route("/transactions", get(get_transactions))
        .route("/transactions/adjustments", get(get_credit_changes))
        .route("/transactions/facets", get(get_transaction_facets))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            account_auth_middleware,
        ))
        .with_state(state)
}

pub fn admin_account_router(state: AccountState) -> Router {
    Router::new()
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", put(update_user))
        .route("/users/{id}/credits", post(adjust_credits))
        .route("/users/{id}/key", post(regenerate_key))
        .merge(
            Router::new()
                .route("/billing", get(get_billing).put(set_billing))
                .with_state(state.clone()),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_middleware,
        ))
        .with_state(state)
}

pub fn login_router(state: AccountState) -> Router {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .with_state(state)
}

/// 公开品牌信息路由（不鉴权），挂在 `/api/branding`
pub fn branding_router(state: AccountState) -> Router {
    Router::new()
        .route("/api/branding", get(get_branding))
        .with_state(state)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BrandingResponse {
    site_name: Option<String>,
    logo_data_url: Option<String>,
}

async fn get_branding(State(state): State<AccountState>) -> Response {
    let config = state.token_manager.as_ref().map(|manager| manager.config());
    Json(BrandingResponse {
        site_name: config.as_ref().and_then(|config| config.site_name.clone()),
        logo_data_url: config.and_then(|config| config.logo_data_url.clone()),
    })
    .into_response()
}

async fn logout(State(state): State<AccountState>, request: axum::extract::Request) -> Response {
    if let Some(token) = crate::common::auth::extract_api_key(&request) {
        let _ = state.store.revoke_web_session(&token);
    }
    Json(SuccessResponse::new("已退出登录")).into_response()
}

async fn get_me(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    respond(state.store.overview(&user))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    username: String,
    password: String,
}

pub async fn login(
    State(state): State<AccountState>,
    Json(request): Json<LoginRequest>,
) -> Response {
    match state
        .store
        .authenticate_password(&request.username, &request.password)
    {
        Ok(Some(user)) => match state.store.create_web_session(&user) {
            Ok(session_token) => match state.store.overview(&user) {
                Ok(account) => {
                    Json(serde_json::json!({ "sessionToken": session_token, "account": account }))
                        .into_response()
                }
                Err(error) => error_response(error),
            },
            Err(error) => error_response(error),
        },
        Ok(None) => (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(AdminErrorResponse::authentication_error()),
        )
            .into_response(),
        Err(error) => error_response(error),
    }
}

async fn change_password(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<ChangePasswordRequest>,
) -> Response {
    match state.store.change_password(&user, &request.current_password, &request.new_password) {
        Ok(api_key) => Json(serde_json::json!({ "success": true, "message": "密码已更新，请使用新密码登录；API Key 已同步轮换", "apiKey": api_key })).into_response(),
        Err(error) => error_response(error),
    }
}

async fn regenerate_own_key(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    match state.store.regenerate_key(&user, user.id) {
        Ok(api_key) => Json(serde_json::json!({ "success": true, "message": "API Key 已重置，旧 Key 已失效", "apiKey": api_key })).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_transactions(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<TransactionQuery>,
) -> Response {
    let filter = match query.filter() {
        Ok(filter) => filter,
        Err(message) => return error_response(AccountError::InvalidInput(message)),
    };
    respond(state.store.list_transactions(
        &user,
        query.user_id,
        &filter,
        query.limit.unwrap_or(100),
    ))
}

async fn get_transaction_facets(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<TransactionFacetsQuery>,
) -> Response {
    respond(state.store.transaction_facets(&user, query.user_id))
}

async fn get_credit_changes(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<TransactionQuery>,
) -> Response {
    respond(
        state
            .store
            .list_credit_changes(&user, query.user_id, query.limit.unwrap_or(100)),
    )
}

async fn list_users(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
) -> Response {
    respond(state.store.list_users(&user))
}

async fn create_user(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<CreateUserRequest>,
) -> Response {
    match state.store.create_user(
        &user,
        &request.username,
        &request.password,
        request.role,
        request.initial_balance,
    ) {
        Ok((user, api_key)) => (
            axum::http::StatusCode::CREATED,
            Json(CreateUserResponse { user, api_key }),
        )
            .into_response(),
        Err(error) => error_response(error),
    }
}

async fn update_user(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(request): Json<UpdateUserRequest>,
) -> Response {
    respond(
        state
            .store
            .update_user(&user, id, request.enabled, request.role),
    )
}

async fn adjust_credits(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(request): Json<AdjustCreditsRequest>,
) -> Response {
    respond(
        state
            .store
            .adjust_credits(&user, id, request.amount, request.note.as_deref()),
    )
}

async fn regenerate_key(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Response {
    match state.store.regenerate_key(&user, id) {
        Ok(api_key) => Json(serde_json::json!({ "success": true, "message": "API Key 已重置，旧 Key 已失效", "apiKey": api_key })).into_response(),
        Err(error) => error_response(error),
    }
}

async fn get_billing(State(state): State<AccountState>) -> Response {
    respond(state.store.get_billing_config())
}

#[axum::debug_handler]
async fn set_billing(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    Json(config): Json<BillingConfig>,
) -> Response {
    if user.role != super::models::Role::SuperAdmin {
        return error_response(AccountError::Forbidden(
            "仅超级管理员可以修改计费单价".into(),
        ));
    }
    match state.store.set_billing_config(&config) {
        Ok(()) => Json(SuccessResponse::new("计费单价已保存")).into_response(),
        Err(error) => error_response(error),
    }
}

fn respond<T: serde::Serialize>(result: Result<T, AccountError>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(error) => error_response(error),
    }
}

#[cfg(all(test, not(feature = "mysql")))]
mod tests {
    use std::sync::Arc;

    use axum::{body::Body, http::Request, http::StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::accounts::AccountStore;
    use crate::kiro::token_manager::MultiTokenManager;
    use crate::model::config::Config;

    async fn get_branding_json(state: AccountState) -> (StatusCode, serde_json::Value) {
        let response = branding_router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/branding")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn branding_is_public_and_reflects_runtime_config() {
        let store = Arc::new(AccountStore::open(":memory:").unwrap());
        let mut config = Config::default();
        config.site_name = Some("我的站点".into());
        config.logo_data_url = Some("data:image/png;base64,iVBORw0KGgo=".into());
        let manager = Arc::new(
            MultiTokenManager::new(
                crate::runtime_config::RuntimeConfig::from_config(config),
                vec![],
                None,
                None,
                false,
            )
            .unwrap(),
        );
        let state = AccountState::new(store, "test-admin-key-123456").with_token_manager(manager);
        let (status, body) = get_branding_json(state).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["siteName"], "我的站点");
        assert_eq!(body["logoDataUrl"], "data:image/png;base64,iVBORw0KGgo=");
    }

    #[tokio::test]
    async fn branding_defaults_to_null_without_config() {
        let store = Arc::new(AccountStore::open(":memory:").unwrap());
        let (status, body) =
            get_branding_json(AccountState::new(store, "test-admin-key-123456")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!({ "siteName": null, "logoDataUrl": null })
        );
    }
}
