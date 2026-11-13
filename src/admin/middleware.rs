//! Admin API 中间件

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

use super::types::AdminErrorResponse;
use super::{core_config, service::AdminService};
use crate::accounts::{AccountState, middleware::authenticate_api_key};
use crate::common::auth;

/// Admin API 共享状态
#[derive(Clone)]
pub struct AdminState {
    /// Admin API 密钥
    pub admin_api_key: String,
    /// Admin 服务
    pub service: Arc<AdminService>,
    /// 核心配置与热重载服务
    pub core_config: Arc<core_config::CoreConfigService>,
    /// 账户认证服务，支持 API Key 与密码登录的网页会话
    pub account_state: AccountState,
}

impl AdminState {
    pub fn new(
        admin_api_key: impl Into<String>,
        service: AdminService,
        account_state: AccountState,
        core_config: core_config::CoreConfigService,
    ) -> Self {
        Self {
            admin_api_key: admin_api_key.into(),
            service: Arc::new(service),
            account_state,
            core_config: Arc::new(core_config),
        }
    }
}

/// Admin API 认证中间件
pub async fn admin_auth_middleware(
    State(state): State<AdminState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(key) = auth::extract_api_key(&request) else {
        let error = AdminErrorResponse::authentication_error();
        return (StatusCode::UNAUTHORIZED, Json(error)).into_response();
    };
    match authenticate_api_key(&state.account_state, &key).await {
        Ok(user) if user.is_manager() => {
            let mut request = request;
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Ok(_) => (
            StatusCode::FORBIDDEN,
            Json(AdminErrorResponse::new("forbidden", "该账户没有管理权限")),
        )
            .into_response(),
        Err(_) => {
            let error = AdminErrorResponse::authentication_error();
            (StatusCode::UNAUTHORIZED, Json(error)).into_response()
        }
    }
}
