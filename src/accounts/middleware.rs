use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

use super::{AccountStore, models::AuthUser, store::AccountError};
use crate::{admin::types::AdminErrorResponse, anthropic::types::ErrorResponse, common::auth};

#[derive(Clone)]
pub struct AccountState {
    pub store: Arc<AccountStore>,
    pub tracker: crate::admin::RequestTracker,
    pub legacy_admin_key: String,
    pub legacy_api_key: Option<String>,
}

impl AccountState {
    pub fn new(store: Arc<AccountStore>, legacy_admin_key: impl Into<String>) -> Self {
        Self {
            store,
            tracker: crate::admin::RequestTracker::new(),
            legacy_admin_key: legacy_admin_key.into(),
            legacy_api_key: None,
        }
    }

    pub fn with_legacy_api_key(mut self, key: impl Into<String>) -> Self {
        self.legacy_api_key = Some(key.into());
        self
    }
}

pub async fn authenticate_api_key(state: &AccountState, key: &str) -> Result<AuthUser, Response> {
    if let Some(user) = state.store.authenticate_key(key).map_err(internal)? {
        return Ok(user);
    }
    if let Some(user) = state
        .store
        .authenticate_web_session(key)
        .map_err(internal)?
    {
        return Ok(user);
    }
    if auth::constant_time_eq(key, &state.legacy_admin_key)
        || state
            .legacy_api_key
            .as_deref()
            .is_some_and(|legacy| auth::constant_time_eq(key, legacy))
    {
        return state.store.super_admin().map_err(internal);
    }
    Err((
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse::authentication_error()),
    )
        .into_response())
}

pub async fn account_auth_middleware(
    State(state): State<AccountState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(key) = auth::extract_api_key(&request) else {
        return unauthorized();
    };
    let user = match authenticate_api_key(&state, &key).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let user = match state.store.authenticate_session(user.id) {
        Ok(user) => user,
        Err(_) => return unauthorized(),
    };
    let mut request = request;
    request.extensions_mut().insert(user);
    next.run(request).await
}

pub async fn admin_auth_middleware(
    State(state): State<AccountState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(key) = auth::extract_api_key(&request) else {
        return unauthorized();
    };
    let user = match authenticate_api_key(&state, &key).await {
        Ok(user) => user,
        Err(_) => return unauthorized(),
    };
    let user = match state.store.authenticate_session(user.id) {
        Ok(user) if user.is_manager() => user,
        _ => {
            return (
                StatusCode::FORBIDDEN,
                Json(AdminErrorResponse::new("forbidden", "该账户没有管理权限")),
            )
                .into_response();
        }
    };
    let mut request = request;
    request.extensions_mut().insert(user);
    next.run(request).await
}

pub async fn super_admin_auth_middleware(
    State(state): State<AccountState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(key) = auth::extract_api_key(&request) else {
        return unauthorized();
    };
    let user = match authenticate_api_key(&state, &key).await {
        Ok(user) => user,
        Err(_) => return unauthorized(),
    };
    let user = match state.store.authenticate_session(user.id) {
        Ok(user) if user.role == super::models::Role::SuperAdmin => user,
        _ => {
            return (
                StatusCode::FORBIDDEN,
                Json(AdminErrorResponse::new(
                    "forbidden",
                    "仅超级管理员可以执行此操作",
                )),
            )
                .into_response();
        }
    };
    let mut request = request;
    request.extensions_mut().insert(user);
    next.run(request).await
}

pub fn error_response(error: AccountError) -> Response {
    let status = match &error {
        AccountError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        AccountError::NotFound => StatusCode::NOT_FOUND,
        AccountError::Conflict(_) => StatusCode::CONFLICT,
        AccountError::Forbidden(_) => StatusCode::FORBIDDEN,
        AccountError::InsufficientCredits { .. } => StatusCode::PAYMENT_REQUIRED,
        AccountError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let message = match &error {
        AccountError::InsufficientCredits {
            required_micro,
            balance_micro,
        } => format!(
            "积分余额不足：本次最多需要 {:.6} 积分，当前余额 {:.6} 积分",
            *required_micro as f64 / 1_000_000.0,
            *balance_micro as f64 / 1_000_000.0
        ),
        _ => error.to_string(),
    };
    if matches!(error, AccountError::Internal(_)) {
        tracing::error!("账户服务错误: {}", message);
    }
    (status, Json(ErrorResponse::new("account_error", message))).into_response()
}

fn internal(error: AccountError) -> Response {
    error_response(error)
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(AdminErrorResponse::authentication_error()),
    )
        .into_response()
}
