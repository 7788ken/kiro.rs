use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::{
    Json, Router,
    body::Body,
    http::{Request, Response, StatusCode},
    middleware::Next,
    response::IntoResponse,
    routing::get,
};
use serde_json::json;

#[derive(Clone)]
pub struct Lifecycle {
    inner: Arc<Inner>,
}

struct Inner {
    draining: AtomicBool,
    active: AtomicUsize,
    notify: tokio::sync::Notify,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                draining: AtomicBool::new(false),
                active: AtomicUsize::new(0),
                notify: tokio::sync::Notify::new(),
            }),
        }
    }
}

impl Lifecycle {
    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::Acquire)
    }

    pub fn active_requests(&self) -> usize {
        self.inner.active.load(Ordering::Acquire)
    }

    pub fn begin_drain(&self) {
        self.inner.draining.store(true, Ordering::Release);
    }

    pub async fn wait_for_drain(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.active_requests() == 0 {
                return;
            }
            notified.await;
        }
    }

    fn status(&self) -> Json<serde_json::Value> {
        let draining = self.is_draining();
        Json(json!({
            "status": if draining { "draining" } else { "ready" },
            "ready": !draining,
            "draining": draining,
            "activeRequests": self.active_requests(),
            "version": env!("CARGO_PKG_VERSION"),
            "instance": std::env::var("KIRO_INSTANCE").unwrap_or_else(|_| "standalone".to_string()),
        }))
    }
}

struct RequestGuard(Lifecycle);

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.0.inner.active.fetch_sub(1, Ordering::AcqRel);
        self.0.inner.notify.notify_waiters();
    }
}

pub async fn track_requests(
    axum::extract::State(lifecycle): axum::extract::State<Lifecycle>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    if lifecycle.is_draining() {
        return (StatusCode::SERVICE_UNAVAILABLE, "instance is draining").into_response();
    }
    lifecycle.inner.active.fetch_add(1, Ordering::AcqRel);
    if lifecycle.is_draining() {
        lifecycle.inner.active.fetch_sub(1, Ordering::AcqRel);
        lifecycle.inner.notify.notify_waiters();
        return (StatusCode::SERVICE_UNAVAILABLE, "instance is draining").into_response();
    }
    let guard = RequestGuard(lifecycle);
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        Body::new(TrackedBody {
            inner: body,
            _guard: guard,
        }),
    )
}

pin_project_lite::pin_project! {
    struct TrackedBody {
        #[pin]
        inner: Body,
        _guard: RequestGuard,
    }
}

impl http_body::Body for TrackedBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        self.project().inner.poll_frame(cx)
    }
}

pub fn router(lifecycle: Lifecycle) -> Router {
    Router::new()
        .route("/health/live", get(|| async { StatusCode::OK }))
        .route(
            "/health/ready",
            get(move || {
                let lifecycle = lifecycle.clone();
                async move {
                    if lifecycle.is_draining() {
                        (StatusCode::SERVICE_UNAVAILABLE, lifecycle.status())
                    } else {
                        (StatusCode::OK, lifecycle.status())
                    }
                }
            }),
        )
}

pub async fn shutdown_signal(lifecycle: Lifecycle) {
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("安装 SIGTERM 处理器失败")
            .recv()
            .await;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate => {},
    }
    lifecycle.begin_drain();
    tracing::info!(
        "收到停止信号：停止接收新请求，等待 {} 个在途响应完成",
        lifecycle.active_requests()
    );
    lifecycle.wait_for_drain().await;
    tracing::info!("所有在途响应已完成，可以安全停止实例");
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn drain_waits_for_stream_end_and_rejects_new_requests() {
        let lifecycle = Lifecycle::default();
        let app = Router::new()
            .route(
                "/",
                get(|| async {
                    Body::from_stream(async_stream::stream! {
                        yield Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"first"));
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        yield Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"last"));
                    })
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                lifecycle.clone(),
                track_requests,
            ));
        let request = Request::builder().uri("/").body(Body::empty()).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(lifecycle.active_requests(), 1);
        let draining = tokio::spawn({
            let lifecycle = lifecycle.clone();
            async move {
                lifecycle.begin_drain();
                lifecycle.wait_for_drain().await;
            }
        });
        tokio::task::yield_now().await;
        assert!(!draining.is_finished());
        let request = Request::builder().uri("/").body(Body::empty()).unwrap();
        let rejected = app.oneshot(request).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"firstlast");
        draining.await.unwrap();
        assert_eq!(lifecycle.active_requests(), 0);
    }
}
