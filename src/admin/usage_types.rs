use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsResponse {
    pub start_date: String,
    pub end_date: String,
    pub time_zone: String,
    pub group_by: String,
    pub total_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub active_users: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub spent_micro: i64,
    pub groups: Vec<UsageGroup>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub key: String,
    pub requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub spent_micro: i64,
    pub average_duration_ms: Option<i64>,
    pub average_first_token_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDaySummary {
    pub date: String,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub spent_micro: i64,
}
