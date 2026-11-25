#![cfg(not(feature = "mysql"))]

use std::sync::Arc;

use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{AccountState, AccountStore, Role};

fn user_fixture() -> (AccountState, String, i64) {
    let store = AccountStore::open(":memory:").unwrap();
    store.ensure_super_admin("test-super-admin-key").unwrap();
    let admin = store
        .authenticate_key("test-super-admin-key")
        .unwrap()
        .unwrap();
    let (_, key) = store
        .create_user(&admin, "test-user", "password-123", Role::User, 1.0)
        .unwrap();
    let id = store.authenticate_key(&key).unwrap().unwrap().id;
    (
        AccountState::new(Arc::new(store), "test-super-admin-key"),
        key,
        id,
    )
}

fn request(key: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("x-api-key", key)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn authenticate(
    axum::extract::State(state): axum::extract::State<AccountState>,
    mut request: Request<Body>,
    next: middleware::Next,
) -> Response {
    let key = request
        .headers()
        .get("x-api-key")
        .unwrap()
        .to_str()
        .unwrap();
    let user = state.store.authenticate_key(key).unwrap().unwrap();
    request.extensions_mut().insert(user);
    next.run(request).await
}

fn app(state: AccountState, response: Response) -> Router {
    let response = Arc::new(parking_lot::Mutex::new(Some(response)));
    Router::new()
        .route(
            "/v1/messages",
            post(move || {
                let response = response.clone();
                async move { response.lock().take().expect("test upstream called once") }
            }),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            super::billing::billing_middleware,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

fn message() -> Value {
    json!({"model":"claude-sonnet-4-6","max_tokens":1000,"messages":[{"role":"user","content":"hello"}]})
}

fn transactions(state: &AccountState, key: &str) -> Vec<super::models::TransactionView> {
    state
        .store
        .list_transactions(
            &state.store.authenticate_key(key).unwrap().unwrap(),
            None,
            &super::models::TransactionFilter::default(),
            20,
        )
        .unwrap()
}

#[tokio::test]
async fn charges_actual_usage_from_json_response() {
    let (state, key, id) = user_fixture();
    let response = Json(json!({"usage":{"input_tokens":100,"output_tokens":100,"credential_credits":0.25}})).into_response();
    let response = app(state.clone(), response)
        .oneshot(request(&key, message()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Some(data) = frame.unwrap().data_ref() {
            bytes.extend_from_slice(data);
        }
    }
    assert!(String::from_utf8_lossy(&bytes).contains("input_tokens"));
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        750_000
    );
    let transactions = transactions(&state, &key);
    assert_eq!(transactions.len(), 1);
    let log = &transactions[0];
    assert_eq!(log.kind, "charge");
    assert_eq!(log.amount_micro, -250_000);
    assert_eq!(log.balance_before_micro, Some(1_000_000));
    assert_eq!(log.balance_after_micro, 750_000);
    assert_eq!(log.refund_micro, -199_990);
    assert_eq!(
        log.note.as_deref(),
        Some("按凭证实际消耗 0.250000 积分结算")
    );
    assert_eq!(log.reserved_micro, 50_010);
    assert_eq!(log.http_status, Some(200));
    assert!(log.duration_ms.is_some());
    assert_eq!(log.path.as_deref(), Some("/v1/messages"));
    assert_eq!(log.request_id.as_deref(), Some(log.id.as_str()));
}

#[tokio::test]
async fn charges_actual_usage_from_sse_response() {
    let (state, key, id) = user_fixture();
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from("event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":100,\"output_tokens\":100,\"credential_credits\":0.25}}\n\n"))
        .unwrap();
    let response = app(state.clone(), response)
        .oneshot(request(&key, json!({"model":"claude-sonnet-4-6","max_tokens":1000,"stream":true,"messages":[{"role":"user","content":"hello"}]})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    while let Some(frame) = body.frame().await {
        frame.unwrap();
    }
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        750_000
    );
    let logs = transactions(&state, &key);
    assert_eq!(logs.len(), 1);
    assert!(logs[0].stream);
    assert!(logs[0].first_token_ms.is_some());
}

#[tokio::test]
async fn records_cache_tokens_without_changing_charge() {
    let (state, key, id) = user_fixture();
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":30,\"cache_read_input_tokens\":60,\"output_tokens\":1}}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":30,\"cache_read_input_tokens\":60,\"output_tokens\":100,\"credential_credits\":0.25}}\n\n",
        )))
        .unwrap();
    let response = app(state.clone(), response)
        .oneshot(request(&key, json!({"model":"claude-sonnet-4-6","max_tokens":1000,"stream":true,"messages":[{"role":"user","content":"hello"}]})))
        .await
        .unwrap();
    response.into_body().collect().await.unwrap();
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        750_000
    );
    let logs = transactions(&state, &key);
    assert_eq!(logs.len(), 1);
    // input_tokens 列记总量 = 普通输入 + 缓存写入 + 缓存读取
    assert_eq!(logs[0].input_tokens, 100);
    assert_eq!(logs[0].output_tokens, 100);
    assert_eq!(logs[0].cache_creation_input_tokens, 30);
    assert_eq!(logs[0].cache_read_input_tokens, 60);
    assert_eq!(logs[0].amount_micro, -250_000);
}

#[tokio::test]
async fn refunds_failed_upstream_request_and_preserves_error() {
    let (state, key, id) = user_fixture();
    let response = (
        StatusCode::BAD_GATEWAY,
        Json(json!({"error":{"type":"api_error","message":"upstream unavailable: credential expired"}})),
    )
        .into_response();
    let response = app(state.clone(), response)
        .oneshot(request(&key, message()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    response.into_body().collect().await.unwrap();
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        1_000_000
    );
    let logs = transactions(&state, &key);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].amount_micro, 0);
    assert_eq!(logs[0].http_status, Some(502));
    assert_eq!(
        logs[0].error_message.as_deref(),
        Some("upstream unavailable: credential expired")
    );
    assert_eq!(logs[0].refund_micro, 50_010);
    assert_eq!(logs[0].reserved_micro, 50_010);
    assert_eq!(logs[0].refund_micro, logs[0].reserved_micro);
    assert_eq!(logs[0].balance_after_micro, 1_000_000);
}

#[tokio::test]
async fn successful_response_without_credential_usage_is_free() {
    let (state, key, id) = user_fixture();
    let response = Json(json!({"usage":{"input_tokens":100,"output_tokens":100}})).into_response();
    let response = app(state.clone(), response)
        .oneshot(request(&key, message()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.into_body().collect().await.unwrap();
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        1_000_000
    );
    let log = &transactions(&state, &key)[0];
    assert_eq!(log.amount_micro, 0);
    assert_eq!(log.refund_micro, log.reserved_micro);
    assert_eq!(
        log.note.as_deref(),
        Some("上游未返回凭证消耗，本次不扣积分")
    );
}

#[tokio::test]
async fn credential_usage_above_balance_drains_to_zero() {
    let (state, key, id) = user_fixture();
    let response = Json(json!({"usage":{"credential_credits":3.0}})).into_response();
    let response = app(state.clone(), response)
        .oneshot(request(&key, message()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.into_body().collect().await.unwrap();
    assert_eq!(state.store.get_user_view(id).unwrap().balance_micro, 0);
    let log = &transactions(&state, &key)[0];
    assert_eq!(log.amount_micro, -1_000_000);
    assert!(log.note.as_deref().unwrap().contains("可用余额不足"));
}

#[tokio::test]
async fn nested_router_preserves_full_request_path() {
    let (state, key, _) = user_fixture();
    let response = Json(json!({"usage":{"input_tokens":100,"output_tokens":100}})).into_response();
    let response = Router::new()
        .merge(app(state.clone(), response))
        .oneshot(request(&key, message()))
        .await
        .unwrap();
    response.into_body().collect().await.unwrap();
    assert_eq!(
        transactions(&state, &key)[0].path.as_deref(),
        Some("/v1/messages")
    );
}

#[tokio::test]
async fn insufficient_balance_returns_402_without_calling_upstream() {
    let (state, key, id) = user_fixture();
    let response = Response::new(Body::from("upstream must not be called"));
    let body = json!({"model":"claude-sonnet-4-6","max_tokens":100000,"messages":[{"role":"user","content":"hello"}]});
    let response = app(state.clone(), response)
        .oneshot(request(&key, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        1_000_000
    );
    let logs = transactions(&state, &key);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].kind, "error");
    assert_eq!(logs[0].amount_micro, 0);
    assert_eq!(logs[0].http_status, Some(402));
    assert!(
        logs[0]
            .error_message
            .as_deref()
            .unwrap()
            .contains("积分余额不足")
    );
}

#[tokio::test]
async fn interrupted_stream_is_an_error_and_refunds_reservation() {
    let (state, key, id) = user_fixture();
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(futures::stream::pending::<
            Result<axum::body::Bytes, std::io::Error>,
        >()))
        .unwrap();
    let response = app(state.clone(), response)
        .oneshot(request(
            &key,
            json!({"model":"claude-sonnet-4-6","max_tokens":1000,"stream":true,"messages":[{"role":"user","content":"hello"}]}),
        ))
        .await
        .unwrap();
    drop(response);
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        1_000_000
    );
    let logs = transactions(&state, &key);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].amount_micro, 0);
    assert_eq!(logs[0].kind, "charge");
    assert_eq!(logs[0].http_status, Some(200));
    assert!(
        logs[0]
            .error_message
            .as_deref()
            .unwrap()
            .contains("客户端中断连接")
    );
    assert_eq!(logs[0].balance_after_micro, 1_000_000);
}

#[tokio::test]
async fn cancel_before_response_is_499_and_refunds_reservation() {
    let (state, key, id) = user_fixture();
    // 上游一直不返回响应头，模拟流式请求等首字时客户端断开
    let app = Router::new()
        .route(
            "/v1/messages",
            post(|| std::future::pending::<Response>()),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            super::billing::billing_middleware,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state.clone());
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        app.oneshot(request(
            &key,
            json!({"model":"claude-sonnet-4-6","max_tokens":1000,"stream":true,"messages":[{"role":"user","content":"hello"}]}),
        )),
    )
    .await;
    assert!(result.is_err(), "handler must still be pending when cancelled");
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        1_000_000
    );
    let logs = transactions(&state, &key);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].kind, "charge");
    assert_eq!(logs[0].amount_micro, 0);
    assert_eq!(logs[0].http_status, Some(499));
    assert_eq!(
        logs[0].error_message.as_deref(),
        Some("客户端在响应开始前取消请求，预扣积分已全额返还")
    );
    assert_eq!(logs[0].refund_micro, logs[0].reserved_micro);
    assert_eq!(logs[0].balance_after_micro, 1_000_000);
}

#[tokio::test]
async fn credit_adjustments_are_not_mixed_into_request_logs() {
    let (state, key, _) = user_fixture();
    let admin = state.store.super_admin().unwrap();
    let user = state.store.authenticate_key(&key).unwrap().unwrap();
    state
        .store
        .adjust_credits(&admin, user.id, 2.0, Some("manual recharge"))
        .unwrap();
    assert_eq!(transactions(&state, &key).len(), 0);
    let changes = state.store.list_credit_changes(&user, None, 20).unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].note.as_deref(), Some("manual recharge"));
}
