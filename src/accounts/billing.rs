use std::sync::Arc;
use std::time::Instant;

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
        AccountStore,
        models::{AuthUser, RequestLog, Reservation},
        store::AccountError,
    },
    token,
};

const ERROR_MESSAGE_MAX_BYTES: usize = 2048;
const ERROR_BODY_MAX_BYTES: usize = 64 * 1024;

pub async fn billing_middleware(
    State(state): State<AccountState>,
    Extension(user): Extension<AuthUser>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if user.is_manager() {
        return next.run(request).await;
    }
    let started_at = Instant::now();
    let path = request
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|uri| uri.0.path().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let path = if path == "/messages" {
        "/v1/messages".to_string()
    } else {
        path
    };
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, 50 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return rejected_request(
                &state,
                &user,
                &path,
                "",
                StatusCode::BAD_REQUEST,
                format!("请求体读取失败: {error}"),
                started_at,
            );
        }
    };
    let payload: Value = match serde_json::from_slice(&bytes) {
        Ok(payload) => payload,
        Err(error) => {
            return rejected_request(
                &state,
                &user,
                &path,
                "",
                StatusCode::BAD_REQUEST,
                format!("请求体必须是有效的 JSON: {error}"),
                started_at,
            );
        }
    };
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let stream = payload
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let max_tokens = match payload.get("max_tokens").and_then(Value::as_i64) {
        Some(value) if value > 0 => value,
        _ => {
            return rejected_request(
                &state,
                &user,
                &path,
                &model,
                StatusCode::BAD_REQUEST,
                "max_tokens 必须为正整数".to_string(),
                started_at,
            );
        }
    };
    let request_payload: crate::anthropic::types::MessagesRequest =
        match serde_json::from_value(payload) {
            Ok(payload) => payload,
            Err(error) => {
                return rejected_request(
                    &state,
                    &user,
                    &path,
                    &model,
                    StatusCode::BAD_REQUEST,
                    format!("请求格式错误: {error}"),
                    started_at,
                );
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
        Err(error) => {
            let http_status = account_error_status(&error);
            let message = account_error_message(&error);
            if matches!(error, AccountError::Internal(_)) {
                tracing::error!("账户服务错误: {}", message);
            }
            state.store.record_rejected_request(
                &user,
                &path,
                &model,
                stream,
                http_status.as_u16(),
                &message,
                elapsed_ms(started_at),
            );
            return error_response(error);
        }
    };
    let mut guard = BillingGuard::new(
        state.store.clone(),
        reservation,
        path,
        model,
        stream,
        started_at,
    );
    let response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    guard.http_status = response.status().as_u16();
    let (response_parts, body) = response.into_parts();
    let is_sse = response_parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let output = stream! {
        let mut body = body;
        let mut transport_error = false;
        while let Some(frame) = body.frame().await {
            let frame: http_body::Frame<Bytes> = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    transport_error = true;
                    guard.record_error(&format!("响应流传输失败: {error}"));
                    yield Err(axum::Error::new(error));
                    break;
                }
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
            if !guard.http_status().is_success() {
                let error = error_message(&guard.buffer);
                guard.record_error(&error);
            }
            guard.buffer.clear();
        }
        guard.settle(guard.http_status().is_success() && !transport_error);
    };
    Response::from_parts(response_parts, Body::from_stream(output))
}

struct BillingGuard {
    store: Arc<AccountStore>,
    reservation: Option<Reservation>,
    path: String,
    model: String,
    stream: bool,
    started_at: Instant,
    http_status: u16,
    first_token_ms: Option<i64>,
    input_tokens: i64,
    output_tokens: i64,
    buffer: Vec<u8>,
    error_message: Option<String>,
    completed: bool,
}

impl BillingGuard {
    fn new(
        store: Arc<AccountStore>,
        reservation: Reservation,
        path: String,
        model: String,
        stream: bool,
        started_at: Instant,
    ) -> Self {
        Self {
            store,
            reservation: Some(reservation),
            path,
            model,
            stream,
            started_at,
            http_status: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            first_token_ms: None,
            input_tokens: 0,
            output_tokens: 0,
            buffer: Vec::new(),
            error_message: None,
            completed: false,
        }
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::from_u16(self.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    fn capture(&mut self, chunk: &[u8]) {
        if self.reservation.is_none() {
            return;
        }
        if self.first_token_ms.is_none() && !chunk.is_empty() {
            self.first_token_ms = Some(elapsed_ms(self.started_at));
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
        let event_type = data.get("type").and_then(Value::as_str);
        if self.first_token_ms.is_none()
            && matches!(
                event_type,
                Some("content_block_start" | "content_block_delta" | "error")
            )
        {
            self.first_token_ms = Some(elapsed_ms(self.started_at));
        }
        if event_type == Some("error") || data.get("error").is_some() {
            let error = data.get("error").unwrap_or(data);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            self.record_error(&message);
        }
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

    fn record_error(&mut self, message: &str) {
        if self.error_message.is_none() {
            self.error_message = Some(truncate_error(message));
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
        if !success && self.error_message.is_none() {
            let message = if self.stream && self.http_status().is_success() {
                "客户端中断连接，本次调用未计费，预扣积分已全额返还".to_string()
            } else {
                format!("请求失败（HTTP {}），本次调用未计费", self.http_status)
            };
            self.record_error(&message);
        }
        if let Some(reservation) = self.reservation.take() {
            let input = if self.input_tokens > 0 {
                self.input_tokens
            } else {
                reservation.input_tokens
            };
            let log = RequestLog {
                path: self.path.clone(),
                model: self.model.clone(),
                stream: self.stream,
                http_status: self.http_status,
                error_message: self.error_message.clone(),
                duration_ms: elapsed_ms(self.started_at),
                first_token_ms: self.first_token_ms,
            };
            if let Err(error) = self.store.settle(
                &reservation,
                success && self.error_message.is_none(),
                input,
                self.output_tokens,
                Some(&log),
            ) {
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

fn elapsed_ms(started_at: Instant) -> i64 {
    started_at.elapsed().as_millis().min(i64::MAX as u128) as i64
}

fn rejected_request(
    state: &AccountState,
    user: &AuthUser,
    path: &str,
    model: &str,
    http_status: StatusCode,
    message: String,
    started_at: Instant,
) -> Response {
    state.store.record_rejected_request(
        user,
        path,
        model,
        false,
        http_status.as_u16(),
        &message,
        elapsed_ms(started_at),
    );
    (http_status, message).into_response()
}

fn account_error_status(error: &AccountError) -> StatusCode {
    match error {
        AccountError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        AccountError::NotFound => StatusCode::NOT_FOUND,
        AccountError::Conflict(_) => StatusCode::CONFLICT,
        AccountError::Forbidden(_) => StatusCode::FORBIDDEN,
        AccountError::InsufficientCredits { .. } => StatusCode::PAYMENT_REQUIRED,
        AccountError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn account_error_message(error: &AccountError) -> String {
    match error {
        AccountError::InsufficientCredits {
            required_micro,
            balance_micro,
        } => format!(
            "积分余额不足：本次最多需要 {:.6} 积分，当前余额 {:.6} 积分",
            *required_micro as f64 / 1_000_000.0,
            *balance_micro as f64 / 1_000_000.0
        ),
        _ => error.to_string(),
    }
}

fn error_message(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(ERROR_BODY_MAX_BYTES)]);
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            return message.to_string();
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return message.to_string();
        }
        if let Some(message) = value.get("error").and_then(Value::as_str) {
            return message.to_string();
        }
        return value.to_string();
    }
    let message = text.trim();
    if message.is_empty() {
        "上游返回错误，但没有提供错误详情".to_string()
    } else {
        message.to_string()
    }
}

fn truncate_error(message: &str) -> String {
    if message.len() <= ERROR_MESSAGE_MAX_BYTES {
        return message.to_string();
    }
    let mut end = ERROR_MESSAGE_MAX_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…（错误内容已截断）", &message[..end])
}
