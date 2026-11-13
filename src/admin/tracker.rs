use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRequest {
    pub request_id: String,
    pub username: String,
    pub model: String,
    pub path: String,
    pub stream: bool,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub http_status: Option<u16>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub error_message: Option<String>,
}

struct TrackedRequest {
    started_at: DateTime<Utc>,
    started: Instant,
    request: LiveRequest,
}

#[derive(Clone)]
pub struct RequestTracker {
    inner: Arc<Mutex<State>>,
}

struct State {
    started: VecDeque<Instant>,
    active: Vec<TrackedRequest>,
    recent: VecDeque<LiveRequest>,
}

impl RequestTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                started: VecDeque::new(),
                active: Vec::new(),
                recent: VecDeque::new(),
            })),
        }
    }

    pub fn begin(
        &self,
        user: &crate::accounts::AuthUser,
        path: &str,
        model: &str,
        stream: bool,
    ) -> TrackedRequestGuard {
        let started_at = Utc::now();
        let started = Instant::now();
        let request = LiveRequest {
            request_id: format!("live_{}", uuid::Uuid::new_v4().simple()),
            username: user.username.clone(),
            model: model.to_string(),
            path: path.to_string(),
            stream,
            started_at,
            finished_at: None,
            http_status: None,
            input_tokens: 0,
            output_tokens: 0,
            duration_ms: None,
            first_token_ms: None,
            error_message: None,
        };
        self.inner.lock().started.push_back(started);
        self.inner.lock().active.push(TrackedRequest {
            started_at,
            started,
            request: request.clone(),
        });
        TrackedRequestGuard {
            tracker: self.clone(),
            request,
            started,
            finished: false,
        }
    }

    pub fn snapshot(&self) -> LiveSnapshot {
        let cutoff = Instant::now() - std::time::Duration::from_secs(60);
        let mut state = self.inner.lock();
        while state
            .started
            .front()
            .is_some_and(|started| *started < cutoff)
        {
            state.started.pop_front();
        }
        let finished: Vec<_> = state
            .active
            .iter()
            .map(|tracked| tracked.request.clone())
            .chain(state.recent.iter().cloned())
            .collect();
        let finished_count = finished
            .iter()
            .filter(|request| request.finished_at.is_some())
            .count();
        let errors = finished
            .iter()
            .filter(|request| {
                request.error_message.is_some()
                    || request.http_status.is_some_and(|status| status >= 400)
            })
            .count();
        LiveSnapshot {
            rpm: state.started.len() as u32,
            active: state.active.len() as u32,
            recent_finished: finished_count as u32,
            success_rate: if finished_count == 0 {
                1.0
            } else {
                (finished_count - errors) as f64 / finished_count as f64
            },
            requests: finished,
        }
    }
}

pub struct LiveSnapshot {
    pub rpm: u32,
    pub active: u32,
    pub recent_finished: u32,
    pub success_rate: f64,
    pub requests: Vec<LiveRequest>,
}

pub struct TrackedRequestGuard {
    tracker: RequestTracker,
    request: LiveRequest,
    started: Instant,
    finished: bool,
}

impl TrackedRequestGuard {
    pub fn request_id(&self) -> &str {
        &self.request.request_id
    }

    pub fn first_token(&mut self) {
        if self.request.first_token_ms.is_none() {
            self.request.first_token_ms = Some(self.started.elapsed().as_millis() as i64);
        }
    }

    pub fn finish(
        mut self,
        status: u16,
        input_tokens: i64,
        output_tokens: i64,
        first_token_ms: Option<i64>,
        error_message: Option<String>,
    ) {
        self.request.finished_at = Some(Utc::now());
        self.request.http_status = Some(status);
        self.request.input_tokens = input_tokens.max(0);
        self.request.output_tokens = output_tokens.max(0);
        self.request.duration_ms = Some(self.started.elapsed().as_millis() as i64);
        self.request.first_token_ms = first_token_ms;
        self.request.error_message = error_message;
        let mut state = self.tracker.inner.lock();
        if let Some(index) = state
            .active
            .iter()
            .position(|tracked| tracked.request.request_id == self.request.request_id)
        {
            state.active.remove(index);
        }
        state.recent.push_front(self.request.clone());
        state.recent.truncate(100);
        self.finished = true;
    }
}

impl Drop for TrackedRequestGuard {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self.tracker.inner.lock();
            if let Some(index) = state
                .active
                .iter()
                .position(|tracked| tracked.request.request_id == self.request.request_id)
            {
                state.active.remove(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AuthUser, Role};

    #[test]
    fn tracks_rpm_active_and_finished_requests() {
        let tracker = RequestTracker::new();
        let user = AuthUser {
            id: 1,
            username: "tester".into(),
            role: Role::User,
            parent_id: None,
            balance_micro: 0,
            enabled: true,
        };
        let first = tracker.begin(&user, "/v1/messages", "sonnet", true);
        tracker
            .begin(&user, "/v1/messages", "sonnet", false)
            .finish(200, 5, 10, None, None);
        assert_eq!(tracker.snapshot().rpm, 2);
        assert_eq!(tracker.snapshot().active, 1);
        assert_eq!(tracker.snapshot().recent_finished, 1);
        first.finish(200, 2, 3, None, None);
        assert_eq!(tracker.snapshot().active, 0);
        assert_eq!(tracker.snapshot().recent_finished, 2);
        assert_eq!(tracker.snapshot().success_rate, 1.0);
    }
}
