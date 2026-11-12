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
use crate::anthropic::types::ErrorResponse;

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

#[tokio::test]
async fn charges_actual_usage_from_json_response() {
    let (state, key, id) = user_fixture();
    let response = Json(json!({"usage":{"input_tokens":100,"output_tokens":100}})).into_response();
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
        994_000
    );
    let transactions = state
        .store
        .list_transactions(
            &state.store.authenticate_key(&key).unwrap().unwrap(),
            None,
            20,
        )
        .unwrap();
    assert!(
        transactions
            .iter()
            .any(|item| item.kind == "charge" && item.amount_micro == -6000)
    );
    assert!(transactions.iter().any(|item| item.kind == "refund"));
}

#[tokio::test]
async fn charges_actual_usage_from_sse_response() {
    let (state, key, id) = user_fixture();
    let response = Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from("event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"input_tokens\":100,\"output_tokens\":100}}\n\n"))
        .unwrap();
    let response = app(state.clone(), response)
        .oneshot(request(&key, message()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    while let Some(frame) = body.frame().await {
        frame.unwrap();
    }
    assert_eq!(
        state.store.get_user_view(id).unwrap().balance_micro,
        994_000
    );
}

#[tokio::test]
async fn refunds_failed_upstream_request() {
    let (state, key, id) = user_fixture();
    let response = (
        StatusCode::BAD_GATEWAY,
        Json(json!({"error":"upstream unavailable"})),
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
    assert!(
        state
            .store
            .list_transactions(
                &state.store.authenticate_key(&key).unwrap().unwrap(),
                None,
                20
            )
            .unwrap()
            .iter()
            .all(|item| item.status == "settled")
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
}
