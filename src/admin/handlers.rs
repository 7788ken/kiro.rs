//! Admin API HTTP 处理器

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::Deserialize;

use super::core_config;
use crate::accounts::AuthUser;

use super::{
    middleware::AdminState,
    types::{
        AddCredentialRequest, ArchiveCredentialRequest, ImportProxiesRequest,
        SetCredentialProxyRequest, SetDisabledRequest,
        SetLoadBalancingModeRequest, SetPriorityRequest, SetProxyEnabledRequest, SuccessResponse,
    },
};
use crate::proxy_pool::ProxyInput;

/// 凭据列表查询参数
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsQuery {
    /// 是否包含已归档凭据（默认 false）
    #[serde(default)]
    pub include_archived: bool,
}

/// GET /api/admin/credentials?includeArchived=true
/// 获取所有凭据状态
pub async fn get_all_credentials(
    State(state): State<AdminState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<CredentialsQuery>,
) -> impl IntoResponse {
    let response = state
        .service
        .get_all_credentials(&user, query.include_archived);
    match response {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/disabled
/// 设置凭据禁用状态
pub async fn set_credential_disabled(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<SetDisabledRequest>,
) -> impl IntoResponse {
    match state.service.set_disabled(&user, id, payload.disabled) {
        Ok(_) => {
            let action = if payload.disabled { "禁用" } else { "启用" };
            Json(SuccessResponse::new(format!("凭据 #{} 已{}", id, action))).into_response()
        }
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/priority
/// 设置凭据优先级
pub async fn set_credential_priority(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<SetPriorityRequest>,
) -> impl IntoResponse {
    match state.service.set_priority(&user, id, payload.priority) {
        Ok(_) => Json(SuccessResponse::new(format!(
            "凭据 #{} 优先级已设置为 {}",
            id, payload.priority
        )))
        .into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/reset
/// 重置失败计数并重新启用
pub async fn reset_failure_count(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    match state.service.reset_and_enable(&user, id) {
        Ok(_) => Json(SuccessResponse::new(format!(
            "凭据 #{} 失败计数已重置并重新启用",
            id
        )))
        .into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// GET /api/admin/credentials/:id/balance
/// 获取指定凭据的余额
pub async fn get_credential_balance(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.service.get_balance(id).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials
/// 添加新凭据
pub async fn add_credential(
    State(state): State<AdminState>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<AddCredentialRequest>,
) -> impl IntoResponse {
    match state.service.add_credential(&user, payload).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/archive
/// 归档凭据（请求体可选：`{"reason": "..."}`）
pub async fn archive_credential(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
    payload: Option<Json<ArchiveCredentialRequest>>,
) -> impl IntoResponse {
    let req = payload.map(|Json(req)| req).unwrap_or_default();
    match state.service.archive_credential(&user, id, req) {
        Ok(_) => Json(SuccessResponse::new(format!("凭据 #{} 已归档", id))).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/unarchive
/// 恢复已归档凭据（恢复后仍为禁用状态）
pub async fn unarchive_credential(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    match state.service.unarchive_credential(&user, id) {
        Ok(_) => Json(SuccessResponse::new(format!(
            "凭据 #{} 已恢复，仍处于禁用状态",
            id
        )))
        .into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// DELETE /api/admin/credentials/:id
/// 彻底删除已归档凭据（仅超级管理员）
pub async fn purge_credential(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    match state.service.purge_credential(&user, id) {
        Ok(_) => Json(SuccessResponse::new(format!("凭据 #{} 已彻底删除", id))).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/refresh
/// 强制刷新凭据 Token
pub async fn force_refresh_token(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    match state.service.force_refresh_token(&user, id).await {
        Ok(_) => Json(SuccessResponse::new(format!(
            "凭据 #{} Token 已强制刷新",
            id
        )))
        .into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// GET /api/admin/config/core
pub async fn get_core_config(
    State(state): State<AdminState>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    if user.role != crate::accounts::Role::SuperAdmin {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(super::types::AdminErrorResponse::new(
                "forbidden",
                "仅超级管理员可以查看核心配置",
            )),
        )
            .into_response();
    }
    Json(state.core_config.get()).into_response()
}

/// PUT /api/admin/config/core
pub async fn update_core_config(
    State(state): State<AdminState>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<core_config::UpdateCoreConfigRequest>,
) -> impl IntoResponse {
    if user.role != crate::accounts::Role::SuperAdmin {
        return (
            axum::http::StatusCode::FORBIDDEN,
            Json(super::types::AdminErrorResponse::new(
                "forbidden",
                "仅超级管理员可以修改核心配置",
            )),
        )
            .into_response();
    }
    match state.core_config.update(&user, payload) {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// GET /api/admin/config/load-balancing
/// 获取负载均衡模式
pub async fn get_load_balancing_mode(State(state): State<AdminState>) -> impl IntoResponse {
    let response = state.service.get_load_balancing_mode();
    Json(response)
}

/// PUT /api/admin/config/load-balancing
/// 设置负载均衡模式
pub async fn set_load_balancing_mode(
    State(state): State<AdminState>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<SetLoadBalancingModeRequest>,
) -> impl IntoResponse {
    match state.service.set_load_balancing_mode(&user, payload) {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// PUT /api/admin/credentials/:id/proxy
/// 修改凭据代理绑定：`{ proxyId }` / `{ direct: true }` / `{ clear: true }` 三选一
pub async fn set_credential_proxy(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Extension(user): Extension<AuthUser>,
    Json(payload): Json<SetCredentialProxyRequest>,
) -> impl IntoResponse {
    match state.service.set_credential_proxy(&user, id, payload) {
        Ok(_) => Json(SuccessResponse::new(format!("凭据 #{} 代理绑定已更新", id))).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/credentials/:id/check-egress
/// 用凭据实际使用的代理检测一次出口 IP（不写入代理池健康状态）
pub async fn check_credential_egress(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.service.check_credential_egress(id).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// GET /api/admin/proxies
pub async fn list_proxies(State(state): State<AdminState>) -> impl IntoResponse {
    Json(state.service.list_proxies())
}

/// POST /api/admin/proxies
pub async fn add_proxy(
    State(state): State<AdminState>,
    Json(payload): Json<ProxyInput>,
) -> impl IntoResponse {
    match state.service.add_proxy(payload) {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// PUT /api/admin/proxies/:id
pub async fn update_proxy(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Json(payload): Json<ProxyInput>,
) -> impl IntoResponse {
    match state.service.update_proxy(id, payload) {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// DELETE /api/admin/proxies/:id
pub async fn delete_proxy(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.service.remove_proxy(id) {
        Ok(_) => Json(SuccessResponse::new(format!("代理 #{} 已删除", id))).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/proxies/:id/enabled
pub async fn set_proxy_enabled(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
    Json(payload): Json<SetProxyEnabledRequest>,
) -> impl IntoResponse {
    match state.service.set_proxy_enabled(id, payload.enabled) {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/proxies/:id/check
pub async fn check_proxy(
    State(state): State<AdminState>,
    Path(id): Path<u64>,
) -> impl IntoResponse {
    match state.service.check_proxy(id).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/proxies/import
/// 批量导入代理（每行一个，空行和 `#` 注释忽略）
pub async fn import_proxies(
    State(state): State<AdminState>,
    Json(payload): Json<ImportProxiesRequest>,
) -> impl IntoResponse {
    match state.service.import_proxies(payload).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => (e.status_code(), Json(e.into_response())).into_response(),
    }
}

/// POST /api/admin/proxies/check-all
pub async fn check_all_proxies(State(state): State<AdminState>) -> impl IntoResponse {
    Json(state.service.check_all_proxies().await)
}
