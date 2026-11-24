//! Admin API 路由配置

use axum::{
    Router, middleware,
    routing::{delete, get, post, put},
};

use super::{
    handlers::{
        add_credential, add_proxy, archive_credential, check_all_proxies,
        check_credential_egress, check_proxy, delete_proxy, force_refresh_token,
        get_all_credentials, get_core_config, get_credential_balance, get_load_balancing_mode,
        list_proxies, purge_credential, reset_failure_count, set_credential_disabled, set_credential_priority,
        set_credential_proxy, set_load_balancing_mode, set_proxy_enabled, unarchive_credential,
        update_core_config, update_proxy,
    },
    middleware::{AdminState, admin_auth_middleware},
};

/// 创建 Admin API 路由
///
/// # 端点
/// - `GET /credentials` - 获取凭据状态（`?includeArchived=true` 包含已归档凭据）
/// - `POST /credentials` - 添加新凭据
/// - `DELETE /credentials/:id` - 彻底删除已归档凭据（仅超级管理员，无法恢复）
/// - `POST /credentials/:id/archive` - 归档凭据
/// - `POST /credentials/:id/unarchive` - 恢复已归档凭据（恢复后仍为禁用）
/// - `POST /credentials/:id/disabled` - 设置凭据禁用状态
/// - `POST /credentials/:id/priority` - 设置凭据优先级
/// - `POST /credentials/:id/reset` - 重置失败计数
/// - `POST /credentials/:id/refresh` - 强制刷新 Token
/// - `GET /credentials/:id/balance` - 获取凭据余额
/// - `PUT /credentials/:id/proxy` - 修改凭据代理绑定（proxyId / direct / clear 三选一）
/// - `POST /credentials/:id/check-egress` - 用凭据实际使用的代理检测出口 IP
/// - `GET|POST /proxies` - 出口代理列表 / 新增
/// - `PUT|DELETE /proxies/:id` - 更新 / 删除出口代理（有未归档凭据绑定时拒绝删除）
/// - `POST /proxies/:id/enabled` - 启用或停用出口代理
/// - `POST /proxies/:id/check` - 检测单个出口代理
/// - `POST /proxies/check-all` - 检测全部启用的出口代理
/// - `GET /config/load-balancing` - 获取负载均衡模式
/// - `PUT /config/load-balancing` - 设置负载均衡模式
///
/// # 认证
/// 需要 Admin API Key 认证，支持：
/// - `x-api-key` header
/// - `Authorization: Bearer <token>` header
pub fn create_admin_router(state: AdminState) -> Router {
    let core = Router::new()
        .route("/config/core", get(get_core_config).put(update_core_config))
        .route(
            "/monitoring/live",
            get(super::monitoring::get_live_requests),
        )
        .route("/monitoring/usage", get(super::monitoring::get_usage_stats))
        .with_state(state.clone());
    let credentials = Router::new()
        .route(
            "/credentials",
            get(get_all_credentials).post(add_credential),
        )
        .route("/credentials/{id}", delete(purge_credential))
        .route("/credentials/{id}/archive", post(archive_credential))
        .route("/credentials/{id}/unarchive", post(unarchive_credential))
        .route("/credentials/{id}/disabled", post(set_credential_disabled))
        .route("/credentials/{id}/priority", post(set_credential_priority))
        .route("/credentials/{id}/reset", post(reset_failure_count))
        .route("/credentials/{id}/refresh", post(force_refresh_token))
        .route("/credentials/{id}/balance", get(get_credential_balance))
        .route("/credentials/{id}/proxy", put(set_credential_proxy))
        .route("/credentials/{id}/check-egress", post(check_credential_egress))
        .route("/proxies", get(list_proxies).post(add_proxy))
        .route("/proxies/check-all", post(check_all_proxies))
        .route("/proxies/{id}", put(update_proxy).delete(delete_proxy))
        .route("/proxies/{id}/enabled", post(set_proxy_enabled))
        .route("/proxies/{id}/check", post(check_proxy))
        .route(
            "/config/load-balancing",
            get(get_load_balancing_mode).put(set_load_balancing_mode),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            admin_auth_middleware,
        ))
        .with_state(state);
    core.merge(credentials)
}
#[cfg(all(test, not(feature = "mysql")))]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::accounts::{AccountState, AccountStore, Role, middleware::admin_auth_middleware};
    use crate::kiro::{model::credentials::KiroCredentials, token_manager::MultiTokenManager};
    use crate::model::config::Config;

    use super::create_admin_router;
    use crate::admin::{AdminService, AdminState};

    struct Fixture {
        store: Arc<AccountStore>,
        app: Router,
        super_admin: crate::accounts::AuthUser,
        proxy_pool: Arc<crate::proxy_pool::ProxyPool>,
    }

    fn fixture() -> Fixture {
        let store = Arc::new(AccountStore::open(":memory:").unwrap());
        store.ensure_super_admin("test-admin-key-123456").unwrap();
        let super_admin = store
            .authenticate_key("test-admin-key-123456")
            .unwrap()
            .unwrap();
        let token_manager = Arc::new(
            MultiTokenManager::new(
                crate::runtime_config::RuntimeConfig::from_config(Config::default()),
                Vec::new(),
                None,
                None,
                false,
            )
            .unwrap(),
        );
        let core_config = crate::admin::core_config::CoreConfigService::new(
            token_manager.runtime_config(),
            token_manager.clone(),
            std::sync::Arc::new(parking_lot::RwLock::new(crate::anthropic::AppState::new(
                "", true,
            ))),
            vec!["ide".into()],
        );
        let proxy_pool = token_manager.proxy_pool();
        let service = AdminService::new(token_manager, ["ide".to_string()], store.clone());
        let account_state = AccountState::new(store.clone(), "test-admin-key-123456");
        let app = create_admin_router(AdminState::new(
            "test-admin-key-123456",
            service,
            account_state.clone(),
            core_config,
        ))
        .layer(middleware::from_fn_with_state(
            account_state,
            admin_auth_middleware,
        ));
        Fixture {
            store,
            app,
            super_admin,
            proxy_pool,
        }
    }

    fn create_admin(
        store: &AccountStore,
        actor: &crate::accounts::AuthUser,
        username: &str,
    ) -> (crate::accounts::AuthUser, String) {
        let (_, key) = store
            .create_user(actor, username, "password-123", Role::Admin, 0.0)
            .unwrap();
        (store.authenticate_key(&key).unwrap().unwrap(), key)
    }

    async fn request(
        app: &Router,
        method: &str,
        uri: &str,
        key: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("x-api-key", key)
                    .header("content-type", "application/json")
                    .body(Body::from(body.unwrap_or_else(|| json!({})).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        )
    }

    async fn add(app: &Router, key: &str, api_key: &str, email: &str) -> u64 {
        let (status, body) = request(
            app,
            "POST",
            "/credentials",
            key,
            Some(json!({
                "authMethod": "api_key",
                "kiroApiKey": api_key,
                "email": email,
                "endpoint": "ide"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["credentialId"].as_u64().unwrap()
    }

    #[tokio::test]
    async fn credential_access_follows_the_complete_admin_branch() {
        let fixture = fixture();
        let (root, root_key) = create_admin(&fixture.store, &fixture.super_admin, "root");
        let (child, child_key) = create_admin(&fixture.store, &root, "child");
        let (_, grandchild_key) = create_admin(&fixture.store, &child, "grandchild");
        assert_eq!(grandchild_key.is_empty(), false);
        let (other, other_key) = create_admin(&fixture.store, &fixture.super_admin, "other");
        let (_, _member_key) = fixture
            .store
            .create_user(&root, "root-member", "password-123", Role::User, 0.0)
            .unwrap();
        fixture
            .store
            .create_user(&other, "other-member", "password-123", Role::User, 0.0)
            .unwrap();
        assert!(
            fixture
                .store
                .list_users(&root)
                .unwrap()
                .iter()
                .all(|user| user.username != "other-member")
        );

        let root_id = add(&fixture.app, &root_key, "ksk_root_test", "root@example.com").await;
        let child_id = add(
            &fixture.app,
            &child_key,
            "ksk_child_test",
            "child@example.com",
        )
        .await;
        let grandchild_id = add(
            &fixture.app,
            &grandchild_key,
            "ksk_grandchild_test",
            "grandchild@example.com",
        )
        .await;
        let other_id = add(
            &fixture.app,
            &other_key,
            "ksk_other_test",
            "other@example.com",
        )
        .await;

        let (status, list) = request(&fixture.app, "GET", "/credentials", &root_key, None).await;
        assert_eq!(status, StatusCode::OK);
        let session = fixture.store.create_web_session(&root).unwrap();
        let (session_status, session_list) =
            request(&fixture.app, "GET", "/credentials", &session, None).await;
        assert_eq!(session_status, StatusCode::OK);
        assert_eq!(session_list["credentials"].as_array().unwrap().len(), 4);
        assert_eq!(list["credentials"].as_array().unwrap().len(), 4);
        for (id, can_manage) in [
            (root_id, true),
            (child_id, true),
            (grandchild_id, true),
            (other_id, false),
        ] {
            let credential = list["credentials"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"].as_u64() == Some(id))
                .unwrap();
            assert_eq!(credential["canManage"].as_bool(), Some(can_manage));
        }

        for id in [root_id, child_id, grandchild_id] {
            let (status, _) = request(
                &fixture.app,
                "POST",
                &format!("/credentials/{id}/priority"),
                &root_key,
                Some(json!({"priority": 7})),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{other_id}/priority"),
            &root_key,
            Some(json!({"priority": 7})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{grandchild_id}/priority"),
            &other_key,
            Some(json!({"priority": 7})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        assert_eq!(other.parent_id, Some(fixture.super_admin.id));
        assert!(
            fixture
                .store
                .can_manage_credential(&root, Some(child.id))
                .unwrap()
        );
        assert!(
            !fixture
                .store
                .can_manage_credential(&other, Some(child.id))
                .unwrap()
        );
    }

    #[tokio::test]
    async fn usage_stats_enforce_permissions_and_reject_invalid_dates() {
        let fixture = fixture();
        let (status, body) = request(
            &fixture.app,
            "GET",
            "/monitoring/usage?startDate=2026-11-13&endDate=2026-11-13&groupBy=day",
            "test-admin-key-123456",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["startDate"], "2026-11-13");
        assert_eq!(body["endDate"], "2026-11-13");
        assert_eq!(body["timeZone"], "Asia/Shanghai");
        assert_eq!(body["activeUsers"], 0);
        let (status, _) = request(
            &fixture.app,
            "GET",
            "/monitoring/usage?startDate=not-a-date",
            "test-admin-key-123456",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = request(
            &fixture.app,
            "GET",
            "/monitoring/usage?startDate=2026-11-13&endDate=2026-11-13&groupBy=other",
            "test-admin-key-123456",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (_, user_key) = fixture
            .store
            .create_user(
                &fixture.super_admin,
                "usage-member",
                "password-123",
                Role::User,
                0.0,
            )
            .unwrap();
        let (status, _) = request(
            &fixture.app,
            "GET",
            "/monitoring/usage?startDate=2026-11-13&endDate=2026-11-13",
            &user_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) = request(
            &fixture.app,
            "GET",
            "/monitoring/live",
            "test-admin-key-123456",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], 0);
    }

    #[tokio::test]
    async fn users_and_legacy_credentials_cannot_be_taken_over() {
        let fixture = fixture();
        let (admin, admin_key) = create_admin(&fixture.store, &fixture.super_admin, "manager");
        let (_, user_key) = fixture
            .store
            .create_user(&admin, "member", "password-123", Role::User, 0.0)
            .unwrap();
        fixture
            .store
            .create_user(
                &fixture.super_admin,
                "legacy-user",
                "password-123",
                Role::User,
                0.0,
            )
            .unwrap();
        assert!(
            !fixture
                .store
                .list_users(&admin)
                .unwrap()
                .iter()
                .any(|user| user.username == "legacy-user")
        );
        assert!(
            fixture
                .store
                .list_users(&admin)
                .unwrap()
                .iter()
                .all(|user| user.id == admin.id
                    || fixture.store.can_manage_account(&admin, user.id).unwrap())
        );

        let (status, _) = request(&fixture.app, "GET", "/credentials", &user_key, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = request(
            &fixture.app,
            "PUT",
            "/config/load-balancing",
            &admin_key,
            Some(json!({"mode": "balanced"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let token_manager = Arc::new(
            MultiTokenManager::new(
                crate::runtime_config::RuntimeConfig::from_config(Config::default()),
                vec![KiroCredentials {
                    id: Some(99),
                    auth_method: Some("api_key".into()),
                    kiro_api_key: Some("ksk_legacy_test".into()),
                    disabled: true,
                    ..Default::default()
                }],
                None,
                None,
                false,
            )
            .unwrap(),
        );
        let service = AdminService::new(
            token_manager.clone(),
            ["ide".to_string()],
            fixture.store.clone(),
        );
        let core_config = crate::admin::core_config::CoreConfigService::new(
            token_manager.runtime_config(),
            token_manager.clone(),
            std::sync::Arc::new(parking_lot::RwLock::new(crate::anthropic::AppState::new(
                "", true,
            ))),
            vec!["ide".into()],
        );
        let app = create_admin_router(AdminState::new(
            "test-admin-key-123456",
            service,
            AccountState::new(fixture.store.clone(), "test-admin-key-123456"),
            core_config,
        ))
        .layer(middleware::from_fn_with_state(
            AccountState::new(fixture.store.clone(), "test-admin-key-123456"),
            admin_auth_middleware,
        ));
        fixture
            .store
            .create_user(
                &fixture.super_admin,
                "legacy-user-2",
                "password-123",
                Role::User,
                0.0,
            )
            .unwrap();
        let (status, list) = request(&app, "GET", "/credentials", &admin_key, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["credentials"][0]["canManage"], false);
        let (status, _) =
            request(&app, "POST", "/credentials/99/archive", &admin_key, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = request(
            &app,
            "POST",
            "/credentials/99/archive",
            "test-admin-key-123456",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    fn find_credential(list: &Value, id: u64) -> Option<&Value> {
        list["credentials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"].as_u64() == Some(id))
    }

    #[tokio::test]
    async fn archive_hides_credential_and_unarchive_keeps_it_disabled() {
        let fixture = fixture();
        let (owner, owner_key) = create_admin(&fixture.store, &fixture.super_admin, "owner");
        let (_, other_key) = create_admin(&fixture.store, &fixture.super_admin, "outsider");
        assert!(owner.id > 0);
        let keep_id = add(&fixture.app, &owner_key, "ksk_keep_test", "keep@example.com").await;
        let id = add(&fixture.app, &owner_key, "ksk_archive_test", "a@example.com").await;

        // 非管理者归档返回 403
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/archive"),
            &other_key,
            Some(json!({"reason": "x"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // 未禁用也可以直接归档
        let (status, body) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/archive"),
            &owner_key,
            Some(json!({"reason": "  不再使用  "})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // 重复归档返回 400
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/archive"),
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // 默认列表不含已归档凭据
        let (status, list) =
            request(&fixture.app, "GET", "/credentials", &owner_key, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(find_credential(&list, id).is_none());
        assert!(find_credential(&list, keep_id).is_some());
        assert_eq!(list["total"], 1);
        assert_eq!(list["available"], 1);

        // includeArchived=true 包含且 archived: true
        let (status, list) = request(
            &fixture.app,
            "GET",
            "/credentials?includeArchived=true",
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let item = find_credential(&list, id).unwrap();
        assert_eq!(item["archived"], true);
        assert_eq!(item["disabled"], true);
        assert_eq!(item["archiveReason"], "不再使用");
        assert_eq!(item["archivedBy"].as_i64(), Some(owner.id));
        assert!(item["archivedAt"].is_string());
        assert_eq!(find_credential(&list, keep_id).unwrap()["archived"], false);

        // 对已归档凭据启用返回 4xx
        let (status, body) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/disabled"),
            &owner_key,
            Some(json!({"disabled": false})),
        )
        .await;
        assert!(status.is_client_error(), "{status} {body}");
        assert!(body.to_string().contains("请先恢复"), "{body}");

        // 恢复：非管理者 403
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/unarchive"),
            &other_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/unarchive"),
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // 恢复后回到默认列表，但仍为禁用
        let (_, list) = request(&fixture.app, "GET", "/credentials", &owner_key, None).await;
        let item = find_credential(&list, id).unwrap();
        assert_eq!(item["archived"], false);
        assert_eq!(item["disabled"], true);
        assert!(item.get("archiveReason").is_none());
        assert_eq!(list["total"], 2);

        // 恢复后可手动启用
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/disabled"),
            &owner_key,
            Some(json!({"disabled": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // DELETE（彻底删除）仅超级管理员可用，普通管理员 403
        let (status, _) = request(
            &fixture.app,
            "DELETE",
            &format!("/credentials/{id}"),
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn purge_requires_super_admin_and_archived_credential() {
        let fixture = fixture();
        let super_key = "test-admin-key-123456";
        let (_, owner_key) = create_admin(&fixture.store, &fixture.super_admin, "owner");
        let keep_id = add(&fixture.app, &owner_key, "ksk_keep_purge", "keep@example.com").await;
        let id = add(&fixture.app, &owner_key, "ksk_purge_test", "p@example.com").await;

        // 普通管理员（即使是登记人）403
        let (status, _) = request(
            &fixture.app,
            "DELETE",
            &format!("/credentials/{id}"),
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // 超级管理员对未归档凭据 400
        let (status, body) = request(
            &fixture.app,
            "DELETE",
            &format!("/credentials/{id}"),
            super_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body.to_string()
                .contains(&format!("只能彻底删除已归档的凭据（请先归档凭据 #{id}）")),
            "{body}"
        );

        // 不存在 404
        let (status, _) =
            request(&fixture.app, "DELETE", "/credentials/999", super_key, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/archive"),
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // 归档后普通管理员仍 403
        let (status, _) = request(
            &fixture.app,
            "DELETE",
            &format!("/credentials/{id}"),
            &owner_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, body) = request(
            &fixture.app,
            "DELETE",
            &format!("/credentials/{id}"),
            super_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, list) = request(
            &fixture.app,
            "GET",
            "/credentials?includeArchived=true",
            super_key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(find_credential(&list, id).is_none());
        assert!(find_credential(&list, keep_id).is_some());
    }

    // ============ 出口代理池 ============

    fn healthy(ip: &str) -> crate::proxy_pool::ProxyHealth {
        crate::proxy_pool::ProxyHealth {
            status: crate::proxy_pool::ProxyStatus::Healthy,
            egress_ip: Some(ip.into()),
            ..Default::default()
        }
    }

    async fn add_proxy(app: &Router, key: &str, name: &str, url: &str) -> u64 {
        let (status, body) = request(
            app,
            "POST",
            "/proxies",
            key,
            Some(json!({"name": name, "url": url})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["id"].as_u64().unwrap()
    }

    async fn add_with(app: &Router, key: &str, api_key: &str, extra: Value) -> (StatusCode, Value) {
        let mut payload = json!({
            "authMethod": "api_key",
            "kiroApiKey": api_key,
            "endpoint": "ide"
        });
        for (k, v) in extra.as_object().unwrap() {
            payload[k] = v.clone();
        }
        request(app, "POST", "/credentials", key, Some(payload)).await
    }

    fn find_proxy(list: &Value, id: u64) -> &Value {
        list["proxies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"].as_u64() == Some(id))
            .unwrap()
    }

    #[tokio::test]
    async fn proxy_crud_hides_password_and_requires_manager() {
        let fixture = fixture();
        let (admin, admin_key) = create_admin(&fixture.store, &fixture.super_admin, "proxy-admin");
        let (_, user_key) = fixture
            .store
            .create_user(&admin, "proxy-member", "password-123", Role::User, 0.0)
            .unwrap();

        // 普通用户 403
        let (status, _) = request(&fixture.app, "GET", "/proxies", &user_key, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = request(
            &fixture.app,
            "POST",
            "/proxies",
            &user_key,
            Some(json!({"name": "x", "url": "http://1.2.3.4:1"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // 新增（普通管理员可用），密码不回显
        let (status, body) = request(
            &fixture.app,
            "POST",
            "/proxies",
            &admin_key,
            Some(json!({
                "name": "东京-1",
                "url": "http://10.0.0.1:3128",
                "username": "proxy-user",
                "password": "proxy-secret-pass",
                "note": "测试"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let id = body["id"].as_u64().unwrap();
        assert_eq!(body["passwordSet"], true);
        assert_eq!(body["boundCount"], 0);
        assert_eq!(body["health"]["status"], "unknown");
        assert!(body.get("password").is_none());
        assert!(!body.to_string().contains("proxy-secret-pass"));

        // URL 校验
        for url in ["ftp://1.2.3.4:21", "not a url", "http://u:p@1.2.3.4:1"] {
            let (status, body) = request(
                &fixture.app,
                "POST",
                "/proxies",
                &admin_key,
                Some(json!({"name": "bad", "url": url})),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{url} {body}");
        }

        // 列表不含密码
        let (status, list) = request(&fixture.app, "GET", "/proxies", &admin_key, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(list["proxies"].as_array().unwrap().len(), 1);
        assert!(!list.to_string().contains("proxy-secret-pass"));

        // 更新：密码留空保留
        let (status, body) = request(
            &fixture.app,
            "PUT",
            &format!("/proxies/{id}"),
            &admin_key,
            Some(json!({
                "name": "东京-2",
                "url": "http://10.0.0.1:3128",
                "username": "proxy-user",
                "password": ""
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["name"], "东京-2");
        assert_eq!(body["passwordSet"], true);
        assert_eq!(
            fixture.proxy_pool.get(id).unwrap().password.as_deref(),
            Some("proxy-secret-pass")
        );

        // 清除密码（同时清用户名）
        let (status, body) = request(
            &fixture.app,
            "PUT",
            &format!("/proxies/{id}"),
            &admin_key,
            Some(json!({"name": "东京-2", "url": "http://10.0.0.1:3128", "clearPassword": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["passwordSet"], false);

        // 启停
        let (status, body) = request(
            &fixture.app,
            "POST",
            &format!("/proxies/{id}/enabled"),
            &admin_key,
            Some(json!({"enabled": false})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["enabled"], false);

        // 不存在的代理 404
        let (status, _) = request(
            &fixture.app,
            "PUT",
            "/proxies/999",
            &admin_key,
            Some(json!({"name": "x", "url": "http://1.2.3.4:1"})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // 删除
        let (status, _) =
            request(&fixture.app, "DELETE", &format!("/proxies/{id}"), &user_key, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) =
            request(&fixture.app, "DELETE", &format!("/proxies/{id}"), &admin_key, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, list) = request(&fixture.app, "GET", "/proxies", &admin_key, None).await;
        assert!(list["proxies"].as_array().unwrap().is_empty());
        let (status, _) =
            request(&fixture.app, "DELETE", &format!("/proxies/{id}"), &admin_key, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn add_credential_auto_assigns_least_bound_healthy_proxy() {
        let fixture = fixture();
        let key = "test-admin-key-123456";

        // 池为空：保持老行为，不绑定
        let (status, body) = add_with(&fixture.app, key, "ksk_pool_none", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let a = add_proxy(&fixture.app, key, "a", "http://10.0.0.1:1").await;
        let b = add_proxy(&fixture.app, key, "b", "http://10.0.0.2:1").await;
        let c = add_proxy(&fixture.app, key, "c", "http://10.0.0.3:1").await;

        // 有启用代理但都未检测 → 400
        let (status, body) = add_with(&fixture.app, key, "ksk_pool_unknown", json!({})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body.to_string()
                .contains("代理池中没有健康的代理，请先检查代理或显式指定 direct"),
            "{body}"
        );
        // 显式 direct 不走自动分配
        let (status, body) = add_with(
            &fixture.app,
            key,
            "ksk_pool_direct",
            json!({"proxyUrl": "direct"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let direct_id = body["credentialId"].as_u64().unwrap();

        fixture.proxy_pool.set_health_for_test(a, healthy("203.0.113.1"));
        fixture.proxy_pool.set_health_for_test(b, healthy("203.0.113.2"));
        // c 保持未知，不参与分配

        let mut assigned = Vec::new();
        for i in 0..3 {
            let (status, body) =
                add_with(&fixture.app, key, &format!("ksk_pool_auto_{i}"), json!({})).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assigned.push(body["credentialId"].as_u64().unwrap());
        }

        let (_, list) = request(&fixture.app, "GET", "/credentials", key, None).await;
        let bound: Vec<Option<u64>> = assigned
            .iter()
            .map(|id| find_credential(&list, *id).unwrap()["proxyId"].as_u64())
            .collect();
        // 0,0 → a；1,0 → b；1,1 → a
        assert_eq!(bound, vec![Some(a), Some(b), Some(a)]);
        let first = find_credential(&list, assigned[0]).unwrap();
        assert_eq!(first["hasProxy"], true);
        assert_eq!(first["proxyName"], "a");
        assert_eq!(first["proxyUrl"], "http://10.0.0.1:1");
        assert_eq!(first["egressIp"], "203.0.113.1");
        let direct = find_credential(&list, direct_id).unwrap();
        assert!(direct.get("proxyId").is_none());
        assert_eq!(direct["proxyUrl"], "direct");

        let (_, proxies) = request(&fixture.app, "GET", "/proxies", key, None).await;
        assert_eq!(find_proxy(&proxies, a)["boundCount"], 2);
        assert_eq!(find_proxy(&proxies, b)["boundCount"], 1);
        assert_eq!(find_proxy(&proxies, c)["boundCount"], 0);

        // 绑定中的代理不能删
        let (status, body) =
            request(&fixture.app, "DELETE", &format!("/proxies/{b}"), key, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.to_string().contains("1 个未归档凭据"), "{body}");
        // 归档后不再计入绑定数，可以删除
        let (status, _) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{}/archive", assigned[1]),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) =
            request(&fixture.app, "DELETE", &format!("/proxies/{b}"), key, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // 显式指定：同时传 proxyId 与 proxyUrl → 400；停用/不存在 → 400；正常 → 绑定
        let (status, _) = add_with(
            &fixture.app,
            key,
            "ksk_pool_both",
            json!({"proxyId": c, "proxyUrl": "http://1.2.3.4:1"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) =
            add_with(&fixture.app, key, "ksk_pool_missing", json!({"proxyId": 999})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        fixture.proxy_pool.set_enabled(c, false).unwrap();
        let (status, _) =
            add_with(&fixture.app, key, "ksk_pool_disabled", json!({"proxyId": c})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        fixture.proxy_pool.set_enabled(c, true).unwrap();
        let (status, body) =
            add_with(&fixture.app, key, "ksk_pool_explicit", json!({"proxyId": c})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let explicit_id = body["credentialId"].as_u64().unwrap();
        let (_, list) = request(&fixture.app, "GET", "/credentials", key, None).await;
        assert_eq!(find_credential(&list, explicit_id).unwrap()["proxyId"].as_u64(), Some(c));
        // 指定的代理未检测（健康未知）时也允许，egressIp 为空
        assert!(find_credential(&list, explicit_id).unwrap().get("egressIp").is_none());
    }

    #[tokio::test]
    async fn rebind_credential_proxy() {
        let fixture = fixture();
        let (owner, owner_key) = create_admin(&fixture.store, &fixture.super_admin, "rebind-owner");
        let (_, other_key) = create_admin(&fixture.store, &fixture.super_admin, "rebind-other");
        assert!(owner.id > 0);
        let (status, body) = add_with(
            &fixture.app,
            &owner_key,
            "ksk_rebind",
            json!({
                "proxyUrl": "http://legacy:1",
                "proxyUsername": "legacy-user",
                "proxyPassword": "legacy-pass"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let id = body["credentialId"].as_u64().unwrap();
        let pool_id = add_proxy(&fixture.app, &owner_key, "p", "socks5://10.0.0.9:1080").await;
        let uri = format!("/credentials/{id}/proxy");

        // 非管理者 403
        let (status, _) =
            request(&fixture.app, "PUT", &uri, &other_key, Some(json!({"proxyId": pool_id}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // 三选一校验
        for bad in [
            json!({}),
            json!({"proxyId": pool_id, "direct": true}),
            json!({"direct": true, "clear": true}),
            json!({"proxyId": 999}),
        ] {
            let (status, body) =
                request(&fixture.app, "PUT", &uri, &owner_key, Some(bad.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} {body}");
        }
        let (status, _) = request(
            &fixture.app,
            "PUT",
            "/credentials/999/proxy",
            &owner_key,
            Some(json!({"direct": true})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // 绑定池条目：清掉直填代理
        let (status, body) =
            request(&fixture.app, "PUT", &uri, &owner_key, Some(json!({"proxyId": pool_id}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, list) = request(&fixture.app, "GET", "/credentials", &owner_key, None).await;
        let item = find_credential(&list, id).unwrap();
        assert_eq!(item["proxyId"].as_u64(), Some(pool_id));
        assert_eq!(item["proxyName"], "p");
        assert_eq!(item["proxyUrl"], "socks5://10.0.0.9:1080");
        assert!(!list.to_string().contains("legacy"));
        let (_, proxies) = request(&fixture.app, "GET", "/proxies", &owner_key, None).await;
        assert_eq!(find_proxy(&proxies, pool_id)["boundCount"], 1);

        // 设为直连
        let (status, _) =
            request(&fixture.app, "PUT", &uri, &owner_key, Some(json!({"direct": true}))).await;
        assert_eq!(status, StatusCode::OK);
        let (_, list) = request(&fixture.app, "GET", "/credentials", &owner_key, None).await;
        let item = find_credential(&list, id).unwrap();
        assert!(item.get("proxyId").is_none());
        assert_eq!(item["proxyUrl"], "direct");
        let (_, proxies) = request(&fixture.app, "GET", "/proxies", &owner_key, None).await;
        assert_eq!(find_proxy(&proxies, pool_id)["boundCount"], 0);

        // 清空：回退全局代理
        let (status, _) =
            request(&fixture.app, "PUT", &uri, &owner_key, Some(json!({"clear": true}))).await;
        assert_eq!(status, StatusCode::OK);
        let (_, list) = request(&fixture.app, "GET", "/credentials", &owner_key, None).await;
        let item = find_credential(&list, id).unwrap();
        assert_eq!(item["hasProxy"], false);
        assert!(item.get("proxyUrl").is_none());
        assert!(item.get("proxyId").is_none());
    }

    #[tokio::test]
    async fn check_egress_uses_resolved_proxy_without_touching_pool_health() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // 假代理：返回固定 IP，请求不会真的出网
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let body = "192.0.2.77";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });

        let fixture = fixture();
        let key = "test-admin-key-123456";
        fixture
            .proxy_pool
            .set_egress_check_url_for_test("http://egress.invalid/");
        let pool_id = add_proxy(&fixture.app, key, "fake", &format!("http://{addr}")).await;
        let (status, body) =
            add_with(&fixture.app, key, "ksk_egress", json!({"proxyId": pool_id})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let id = body["credentialId"].as_u64().unwrap();

        let (status, body) = request(
            &fixture.app,
            "POST",
            &format!("/credentials/{id}/check-egress"),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["health"]["status"], "healthy");
        assert_eq!(body["health"]["egressIp"], "192.0.2.77");
        assert_eq!(body["proxyUrl"], format!("http://{addr}"));
        // 不写入池健康
        assert_eq!(
            fixture.proxy_pool.health_of(pool_id).status,
            crate::proxy_pool::ProxyStatus::Unknown
        );

        // 池检测会写入
        let (status, body) = request(
            &fixture.app,
            "POST",
            &format!("/proxies/{pool_id}/check"),
            key,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["health"]["egressIp"], "192.0.2.77");
        let (status, body) = request(&fixture.app, "POST", "/proxies/check-all", key, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(find_proxy(&body, pool_id)["health"]["status"], "healthy");
    }
}
