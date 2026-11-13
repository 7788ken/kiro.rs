use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Duration, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use serde::Deserialize;

use crate::accounts::AuthUser;

use super::{middleware::AdminState, types::AdminErrorResponse};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub group_by: Option<String>,
    pub user_id: Option<i64>,
    pub limit: Option<u32>,
}

pub async fn get_live_requests(
    State(state): State<AdminState>,
    Extension(_user): Extension<AuthUser>,
) -> impl IntoResponse {
    let snapshot = state.account_state.tracker.snapshot();
    Json(serde_json::json!({
        "rpm": snapshot.rpm,
        "active": snapshot.active,
        "recentFinished": snapshot.recent_finished,
        "successRate": snapshot.success_rate,
        "requests": snapshot.requests,
        "serverTime": Utc::now(),
    }))
}

pub async fn get_usage_stats(
    State(state): State<AdminState>,
    Extension(user): Extension<AuthUser>,
    Query(query): Query<UsageQuery>,
) -> Response {
    let group = query.group_by.as_deref().unwrap_or("day");
    if !["day", "user", "model", "status"].contains(&group) {
        return invalid_request("统计维度必须是 day、user、model 或 status");
    }
    let today = shanghai_date(Utc::now());
    let start_date = match query.start_date.as_deref().map(parse_date).transpose() {
        Ok(date) => date.unwrap_or(today - Duration::days(30)),
        Err(response) => return response,
    };
    let end_date = match query.end_date.as_deref().map(parse_date).transpose() {
        Ok(date) => date.unwrap_or(today),
        Err(response) => return response,
    };
    if start_date > end_date {
        return invalid_request("开始日期不能晚于结束日期");
    }
    let Some(end_of_range) = end_date.succ_opt() else {
        return invalid_request("结束日期超出支持范围");
    };
    let start = utc_boundary(start_date);
    let end = utc_boundary(end_of_range);
    match state.account_state.store.usage_stats(
        &user,
        &start,
        &end,
        group,
        query.user_id,
        query.limit.unwrap_or(100),
    ) {
        Ok(mut stats) => {
            stats.start_date = start_date.to_string();
            stats.end_date = end_date.to_string();
            Json(stats).into_response()
        }
        Err(error) => crate::accounts::middleware::error_response(error).into_response(),
    }
}

fn parse_date(value: &str) -> Result<NaiveDate, Response> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| invalid_request("日期格式必须为 YYYY-MM-DD"))
}

fn invalid_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(AdminErrorResponse::invalid_request(message)),
    )
        .into_response()
}

fn shanghai_date(time: chrono::DateTime<Utc>) -> NaiveDate {
    (time + Duration::hours(8)).date_naive()
}

fn utc_boundary(date: NaiveDate) -> String {
    FixedOffset::east_opt(8 * 3600)
        .unwrap()
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc)
        .with_nanosecond(0)
        .unwrap()
        .to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_dates_use_beijing_days_and_exclusive_end() {
        assert_eq!(
            utc_boundary(NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()),
            "2026-11-12T16:00:00+00:00"
        );
        assert_eq!(
            utc_boundary(NaiveDate::from_ymd_opt(2026, 11, 14).unwrap()),
            "2026-11-13T16:00:00+00:00"
        );
        assert_eq!(
            shanghai_date(Utc::now()),
            (Utc::now() + Duration::hours(8)).date_naive()
        );
    }
}
