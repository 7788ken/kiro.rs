//! Admin API 路由配置

use axum::{
    Router, middleware,
    routing::{delete, get, post},
};

use super::{
    handlers::{
        add_credential, delete_credential, force_refresh_token, get_all_credentials,
        get_core_config, get_credential_balance, get_load_balancing_mode, reset_failure_count,
        set_credential_disabled, set_credential_priority, set_load_balancing_mode,
        update_core_config,
    },
    middleware::{AdminState, admin_auth_middleware},
};

/// 创建 Admin API 路由
///
/// # 端点
/// - `GET /credentials` - 获取所有凭据状态
/// - `POST /credentials` - 添加新凭据
/// - `DELETE /credentials/:id` - 删除凭据
/// - `POST /credentials/:id/disabled` - 设置凭据禁用状态
/// - `POST /credentials/:id/priority` - 设置凭据优先级
/// - `POST /credentials/:id/reset` - 重置失败计数
/// - `POST /credentials/:id/refresh` - 强制刷新 Token
/// - `GET /credentials/:id/balance` - 获取凭据余额
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
        .route("/credentials/{id}", delete(delete_credential))
        .route("/credentials/{id}/disabled", post(set_credential_disabled))
        .route("/credentials/{id}/priority", post(set_credential_priority))
        .route("/credentials/{id}/reset", post(reset_failure_count))
        .route("/credentials/{id}/refresh", post(force_refresh_token))
        .route("/credentials/{id}/balance", get(get_credential_balance))
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
        let (status, _) = request(&app, "DELETE", "/credentials/99", &admin_key, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = request(
            &app,
            "DELETE",
            "/credentials/99",
            "test-admin-key-123456",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
}
