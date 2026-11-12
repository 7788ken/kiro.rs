use std::sync::Arc;

use async_stream::stream;
use axum::{
    Extension,
    body::{Body, Bytes},
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use serde_json::Value;

use crate::{
    accounts::{
        AccountState,
        middleware::error_response,
        models::{AuthUser, Reservation},
        store::AccountStore,
    },
    token,
};

pub async fn billing_middleware(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if user.is_manager() {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 50 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求体读取失败").into_response(),
    };
    let payload: Value = match serde_json::from_slice(&bytes) {
        Ok(payload) => payload,
        Err(_) => return (StatusCode::BAD_REQUEST, "请求体必须是有效的 JSON").into_response(),
    };
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let max_tokens = match payload.get("max_tokens").and_then(Value::as_i64) {
        Some(value) if value > 0 => value,
        _ => return (StatusCode::BAD_REQUEST, "max_tokens 必须为正整数").into_response(),
    };
    let request_payload: crate::anthropic::types::MessagesRequest =
        match serde_json::from_value(payload) {
            Ok(payload) => payload,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, format!("请求格式错误: {error}")).into_response();
            }
        };
    let input_tokens = token::count_all_tokens_local(
        request_payload.system,
        request_payload.messages,
        request_payload.tools,
    ) as i64;
    let reservation = match state.store.reserve(&user, &model, input_tokens, max_tokens) {
        Ok(Some(reservation)) => reservation,
        Ok(None) => {
            return next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await;
        }
        Err(error) => return error_response(error),
    };
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let upstream_status = response.status();
    let (response_parts, body) = response.into_parts();
    let mut guard = BillingGuard::new(state.store.clone(), reservation);
    let is_sse = response_parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let output = stream! {
        let mut body = body;
        while let Some(frame) = body.frame().await {
            let frame: http_body::Frame<Bytes> = match frame {
                Ok(frame) => frame,
                Err(error) => Err(axum::Error::new(error))?,
            };
            let data = match frame.into_data() {
                Ok(data) => data,
                Err(_) => continue,
            };
            if is_sse {
                guard.capture(&data);
            } else {
                guard.buffer.extend_from_slice(&data);
            }
            yield Ok::<_, axum::Error>(data);
        }
        if !is_sse {
            if let Ok(data) = serde_json::from_slice::<Value>(&guard.buffer) {
                guard.capture_json(&data);
            }
            guard.buffer.clear();
        }
        guard.settle(upstream_status.is_success());
    };
    Response::from_parts(response_parts, Body::from_stream(output))
}

struct BillingGuard {
    store: Arc<AccountStore>,
    reservation: Option<Reservation>,
    input_tokens: i64,
    output_tokens: i64,
    buffer: Vec<u8>,
    completed: bool,
}

impl BillingGuard {
    fn new(store: Arc<AccountStore>, reservation: Reservation) -> Self {
        Self {
            store,
            reservation: Some(reservation),
            input_tokens: 0,
            output_tokens: 0,
            buffer: Vec::new(),
            completed: false,
        }
    }

    fn capture(&mut self, chunk: &[u8]) {
        if self.reservation.is_none() {
            return;
        }
        self.buffer.extend_from_slice(chunk);
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=position).collect::<Vec<_>>();
            let Ok(line) = std::str::from_utf8(&line) else {
                continue;
            };
            let Some(data) = line.trim().strip_prefix("data:") else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            self.capture_json(&data);
        }
    }

    fn capture_json(&mut self, data: &Value) {
        if data.get("type").and_then(Value::as_str) == Some("message") {
            self.capture_usage(data);
        }
        if let Some(usage) = data.get("usage") {
            self.capture_usage(usage);
        }
        if let Some(usage) = data.pointer("/message/usage") {
            self.capture_usage(usage);
        }
        if let Some(input) = data.get("input_tokens").and_then(Value::as_i64) {
            self.input_tokens = self.input_tokens.max(input.max(0));
        }
        if let Some(output) = data.get("output_tokens").and_then(Value::as_i64) {
            self.output_tokens = self.output_tokens.max(output.max(0));
        }
    }

    fn capture_usage(&mut self, usage: &Value) {
        if let Some(input) = usage.get("input_tokens").and_then(Value::as_i64) {
            self.input_tokens = self.input_tokens.max(input.max(0));
        }
        if let Some(output) = usage.get("output_tokens").and_then(Value::as_i64) {
            self.output_tokens = self.output_tokens.max(output.max(0));
        }
    }

    fn settle(&mut self, success: bool) {
        if self.completed {
            return;
        }
        self.completed = true;
        if !self.buffer.is_empty() {
            if let Ok(line) = std::str::from_utf8(&self.buffer) {
                if let Some(data) = line.trim().strip_prefix("data:") {
                    if let Ok(data) = serde_json::from_str::<Value>(data.trim()) {
                        self.capture_json(&data);
                    }
                }
            }
            self.buffer.clear();
        }
        if let Some(reservation) = self.reservation.take() {
            let input = if self.input_tokens > 0 {
                self.input_tokens
            } else {
                reservation.input_tokens
            };
            if let Err(error) = self
                .store
                .settle(&reservation, success, input, self.output_tokens)
            {
                tracing::error!(reservation = %reservation.id, "结算积分失败: {}", error);
            }
        }
    }
}

impl Drop for BillingGuard {
    fn drop(&mut self) {
        self.settle(false);
    }
}
