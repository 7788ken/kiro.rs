//! 计费事件
//!
//! Kiro `meteringEvent` 的 payload 形如
//! `{"unit":"credit","unitPlural":"credits","usage":0.016954}`。
//! `usage` 是本次请求消耗的 credit，不是 token。

use serde::Deserialize;

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::EventPayload;

/// 上游凭证的单次消耗
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeteringEvent {
    /// 计量单位，正常为 `credit`
    #[serde(default)]
    pub unit: Option<String>,
    /// 复数单位，正常为 `credits`
    #[serde(default)]
    pub unit_plural: Option<String>,
    /// 本次请求消耗的 credit
    #[serde(default)]
    pub usage: Option<f64>,
}

impl EventPayload for MeteringEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }
}

impl MeteringEvent {
    /// 换算成可入账的 credit。
    ///
    /// 缺字段、非有限值、负数，或单位不是 credit 时返回 `None`，
    /// 调用方不得用 Token 单价代替。
    pub fn credit_usage(&self) -> Option<f64> {
        let usage = self.usage?;
        if !usage.is_finite() || usage < 0.0 {
            return None;
        }
        match self.unit.as_deref() {
            None | Some("credit") | Some("credits") => Some(usage),
            Some(_) => None,
        }
    }
}

impl std::fmt::Display for MeteringEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.credit_usage() {
            Some(usage) => write!(
                f,
                "{usage} {}",
                self.unit_plural.as_deref().unwrap_or("credits")
            ),
            None => write!(f, "无可用消耗"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_credit_payload() {
        let event: MeteringEvent = serde_json::from_str(
            r#"{"unit":"credit","unitPlural":"credits","usage":0.0169543708291874}"#,
        )
        .unwrap();
        let usage = event.credit_usage().unwrap();
        assert!((usage - 0.0169543708291874).abs() < 1e-12);
    }

    #[test]
    fn missing_or_non_credit_usage_is_not_billable() {
        let missing: MeteringEvent =
            serde_json::from_str(r#"{"unit":"credit"}"#).unwrap();
        assert!(missing.credit_usage().is_none());
        let other: MeteringEvent = serde_json::from_str(
            r#"{"unit":"token","unitPlural":"tokens","usage":12}"#,
        )
        .unwrap();
        assert!(other.credit_usage().is_none());
        let zero: MeteringEvent =
            serde_json::from_str(r#"{"usage":0}"#).unwrap();
        assert_eq!(zero.credit_usage(), Some(0.0));
    }
}
