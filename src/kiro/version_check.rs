//! Kiro 客户端版本检测
//!
//! 比对配置的 `kiroVersion` 与官方最新稳定版、官方最低支持版本：
//! - 最新稳定版：官方桌面端更新元数据（无需认证）
//! - 最低支持版本：官方无接口，使用代码内经人工审核的静态清单
//!
//! 检测结果只缓存在内存中，不修改任何配置。

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{FixedOffset, Utc};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::http_client::build_client;
use crate::runtime_config::{RuntimeConfig, proxy_from_config};

/// 官方最新稳定版元数据地址
pub const LATEST_METADATA_URL: &str =
    "https://prod.download.desktop.kiro.dev/stable/metadata-win32-x64-user-stable.json";

/// 拉取元数据的超时（秒）
const FETCH_TIMEOUT_SECS: u64 = 5;

/// 手动检测防抖窗口
const MANUAL_REFRESH_THROTTLE: Duration = Duration::from_secs(60);

/// 后台定时检测间隔
pub const VERSION_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// 经审核的官方最低支持版本条目
#[derive(Debug, Clone, Copy)]
pub struct MinVersionNotice {
    pub min_version: &'static str,
    pub effective_date: &'static str,
    pub source_url: &'static str,
    pub summary: &'static str,
}

/// 官方最低支持版本清单（静态，需人工审核后更新；按生效日期升序，取最后一条）
pub const MIN_VERSION_NOTICES: &[MinVersionNotice] = &[MinVersionNotice {
    min_version: "0.11.133",
    effective_date: "2026-11-09",
    source_url: "https://github.com/kirodotdev/Kiro/issues/11335",
    summary: "官方公告：2026-11-09 起 IDE 低于 0.11.133 无法连接",
}];

fn current_min_notice() -> &'static MinVersionNotice {
    MIN_VERSION_NOTICES
        .last()
        .expect("最低支持版本清单不能为空")
}

/// 版本比对状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionStatus {
    /// 不低于最低支持版本，且不落后于最新版（或最新版尚未获取）
    Ok,
    /// 不低于最低支持版本，但低于官方最新版
    BehindLatest,
    /// 低于最低支持版本，或上游已明确拒绝当前版本
    Unsupported,
    /// 无法比较（配置版本无法解析）
    Unknown,
}

/// 版本检测结果（对外接口结构）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionCheckState {
    pub configured_version: String,
    pub latest_version: Option<String>,
    pub latest_pub_date: Option<String>,
    pub min_version: String,
    pub min_source_url: String,
    pub min_effective_date: String,
    pub min_summary: String,
    pub checked_at: Option<String>,
    pub last_error: Option<String>,
    pub status: VersionStatus,
}

/// 手动检测响应
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRefreshResponse {
    #[serde(flatten)]
    pub state: VersionCheckState,
    pub throttled: bool,
}

#[derive(Debug, Default)]
struct Cached {
    latest_version: Option<String>,
    latest_pub_date: Option<String>,
    checked_at: Option<String>,
    fetch_error: Option<String>,
    /// 上游以 400 拒绝当前版本时的记录（含时间）
    upstream_rejection: Option<String>,
    /// 被上游拒绝时配置的 kiroVersion
    rejected_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    current_release: String,
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    #[serde(default)]
    update_to: Option<UpdateTo>,
}

#[derive(Debug, Deserialize)]
struct UpdateTo {
    #[serde(default)]
    pub_date: Option<String>,
}

/// 版本检测服务
pub struct VersionCheckService {
    runtime: Arc<RuntimeConfig>,
    metadata_url: String,
    cached: RwLock<Cached>,
    /// 最近一次检测（含失败）的时刻，用于手动检测防抖
    last_attempt: Mutex<Option<Instant>>,
    /// 串行化检测，避免并发拉取
    refresh_lock: tokio::sync::Mutex<()>,
}

impl VersionCheckService {
    pub fn new(runtime: Arc<RuntimeConfig>) -> Self {
        Self::with_metadata_url(runtime, LATEST_METADATA_URL)
    }

    pub fn with_metadata_url(runtime: Arc<RuntimeConfig>, url: impl Into<String>) -> Self {
        Self {
            runtime,
            metadata_url: url.into(),
            cached: RwLock::new(Cached::default()),
            last_attempt: Mutex::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// 当前缓存状态；配置版本读运行时配置当前值，修改后立即反映
    pub fn state(&self) -> VersionCheckState {
        let configured = self.runtime.get().kiro_version.clone();
        let cached = self.cached.read();
        let notice = current_min_notice();
        let (status, compare_error) = evaluate(
            &configured,
            notice.min_version,
            cached.latest_version.as_deref(),
            cached.upstream_rejection.is_some(),
        );
        // 被拒绝的版本不低于本地清单的最低版本：清单可能已过期
        let stale_notice_hint = cached
            .upstream_rejection
            .as_ref()
            .and(cached.rejected_version.as_deref())
            .filter(|rejected| {
                matches!(
                    compare_versions(notice.min_version, rejected),
                    Ok(Ordering::Less | Ordering::Equal)
                )
            })
            .map(|_| {
                format!(
                    "官方最低支持版本可能已高于本地清单 {}，请核对官方公告",
                    notice.min_version
                )
            });
        let errors: Vec<&str> = [
            cached.upstream_rejection.as_deref(),
            stale_notice_hint.as_deref(),
            cached.fetch_error.as_deref(),
            compare_error.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        VersionCheckState {
            configured_version: configured,
            latest_version: cached.latest_version.clone(),
            latest_pub_date: cached.latest_pub_date.clone(),
            min_version: notice.min_version.to_string(),
            min_source_url: notice.source_url.to_string(),
            min_effective_date: notice.effective_date.to_string(),
            min_summary: notice.summary.to_string(),
            checked_at: cached.checked_at.clone(),
            last_error: (!errors.is_empty()).then(|| errors.join("；")),
            status,
        }
    }

    /// 后台定时检测：无防抖
    pub async fn check_now(&self) -> VersionCheckState {
        let _guard = self.refresh_lock.lock().await;
        self.run_check().await;
        self.state()
    }

    /// 管理员手动检测：60 秒内已检测过则直接返回缓存
    pub async fn refresh_manual(&self) -> VersionRefreshResponse {
        let _guard = self.refresh_lock.lock().await;
        let throttled = self
            .last_attempt
            .lock()
            .is_some_and(|at| at.elapsed() < MANUAL_REFRESH_THROTTLE);
        if !throttled {
            self.run_check().await;
        }
        VersionRefreshResponse {
            state: self.state(),
            throttled,
        }
    }

    async fn run_check(&self) {
        *self.last_attempt.lock() = Some(Instant::now());
        let result = self.fetch_latest().await;
        let checked_at = Utc::now().to_rfc3339();
        let mut cached = self.cached.write();
        cached.checked_at = Some(checked_at);
        match result {
            Ok((version, pub_date)) => {
                cached.latest_version = Some(version);
                cached.latest_pub_date = pub_date;
                cached.fetch_error = None;
                // 上游拒绝标记：成功检测后，仅当配置版本相对被拒绝时已改动、
                // 且不低于最低支持版本才清除（静态清单可能过期，版本未改动不清除）
                if cached.upstream_rejection.is_some() {
                    let configured = self.runtime.get().kiro_version.clone();
                    let changed = cached.rejected_version.as_deref() != Some(configured.as_str());
                    if changed
                        && matches!(
                            compare_versions(&configured, current_min_notice().min_version),
                            Ok(Ordering::Greater | Ordering::Equal)
                        )
                    {
                        cached.upstream_rejection = None;
                        cached.rejected_version = None;
                    }
                }
            }
            Err(error) => {
                tracing::warn!("Kiro 最新版本检测失败: {}", error);
                cached.fetch_error = Some(format!("获取官方最新版本失败：{error}"));
            }
        }
    }

    async fn fetch_latest(&self) -> anyhow::Result<(String, Option<String>)> {
        let config = self.runtime.get();
        let client = build_client(
            proxy_from_config(&config).as_ref(),
            FETCH_TIMEOUT_SECS,
            config.tls_backend,
        )?;
        let response = client.get(&self.metadata_url).send().await?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("HTTP {}", status);
        }
        let body = response.text().await?;
        parse_metadata(&body)
    }

    /// 上游 400 响应联动：body 含 "no longer supported" 时标记为不受支持。
    /// 返回是否命中。
    pub fn note_upstream_rejection(&self, body: &str) -> bool {
        if !is_version_unsupported_body(body) {
            return false;
        }
        let now = Utc::now()
            .with_timezone(&FixedOffset::east_opt(8 * 3600).expect("UTC+8 偏移合法"))
            .format("%Y-%m-%d %H:%M:%S UTC+8");
        let rejected_version = self.runtime.get().kiro_version.clone();
        let mut cached = self.cached.write();
        if cached.upstream_rejection.is_none() {
            tracing::warn!(
                "上游拒绝当前 Kiro 版本 {}，请在后台核心配置中调整 kiroVersion",
                rejected_version
            );
        }
        cached.rejected_version = Some(rejected_version);
        cached.upstream_rejection = Some(format!(
            "上游拒绝：This version of Kiro is no longer supported（{now}）"
        ));
        true
    }
}

/// 上游 400 body 是否为「版本不再受支持」
pub fn is_version_unsupported_body(body: &str) -> bool {
    body.to_ascii_lowercase().contains("no longer supported")
}

fn parse_metadata(body: &str) -> anyhow::Result<(String, Option<String>)> {
    let metadata: Metadata =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("元数据格式无法解析: {e}"))?;
    let version = metadata.current_release.trim().to_string();
    parse_version(&version).map_err(|e| anyhow::anyhow!("官方最新版本号无效: {e}"))?;
    let pub_date = metadata
        .releases
        .into_iter()
        .next()
        .and_then(|release| release.update_to)
        .and_then(|update| update.pub_date);
    Ok((version, pub_date))
}

/// 解析数字分段版本号，如 `0.11.133`；任意段数，每段必须为非负整数
pub fn parse_version(version: &str) -> Result<Vec<u64>, String> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Err("版本号为空".into());
    }
    trimmed
        .split('.')
        .map(|segment| {
            if segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("版本号 {trimmed:?} 不是数字分段格式"));
            }
            segment
                .parse::<u64>()
                .map_err(|_| format!("版本号 {trimmed:?} 分段过大"))
        })
        .collect()
}

/// 按数字分段比较版本；段数不同时缺省段视为 0（`1.5` == `1.5.0`）
pub fn compare_versions(a: &str, b: &str) -> Result<Ordering, String> {
    let a = parse_version(a)?;
    let b = parse_version(b)?;
    let len = a.len().max(b.len());
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => continue,
            other => return Ok(other),
        }
    }
    Ok(Ordering::Equal)
}

/// 计算状态；返回 (状态, 比较错误)
fn evaluate(
    configured: &str,
    min_version: &str,
    latest: Option<&str>,
    upstream_rejected: bool,
) -> (VersionStatus, Option<String>) {
    if upstream_rejected {
        return (VersionStatus::Unsupported, None);
    }
    match compare_versions(configured, min_version) {
        Err(error) => (
            VersionStatus::Unknown,
            Some(format!("无法比较配置版本：{error}")),
        ),
        Ok(Ordering::Less) => (VersionStatus::Unsupported, None),
        Ok(_) => match latest {
            None => (VersionStatus::Ok, None),
            Some(latest) => match compare_versions(configured, latest) {
                Ok(Ordering::Less) => (VersionStatus::BehindLatest, None),
                Ok(_) => (VersionStatus::Ok, None),
                Err(error) => (
                    VersionStatus::Ok,
                    Some(format!("无法与最新版本比较：{error}")),
                ),
            },
        },
    }
}

/// 后台定时检测：启动后立即检测一次，之后每 24 小时一次。
/// 单轮检测 panic 只记录日志，不影响主服务和后续轮次。
pub fn spawn_version_checker(service: Arc<VersionCheckService>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(VERSION_CHECK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let service = service.clone();
            match tokio::spawn(async move { service.check_now().await }).await {
                Ok(state) => tracing::info!(
                    "Kiro 版本检测完成：配置 {}，最新 {}，最低 {}，状态 {:?}",
                    state.configured_version,
                    state.latest_version.as_deref().unwrap_or("未知"),
                    state.min_version,
                    state.status
                ),
                Err(e) => tracing::warn!("Kiro 版本检测任务异常: {}", e),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::config::Config;

    const METADATA: &str = r#"{"currentRelease":"1.5.37","releases":[{"version":"1.5.37","updateTo":{"version":"1.5.37","pub_date":"2026-11-24","url":"https://example.invalid"}}]}"#;

    fn runtime(version: &str) -> Arc<RuntimeConfig> {
        let mut config = Config::default();
        config.kiro_version = version.into();
        Arc::new(RuntimeConfig::new(
            config,
            "/tmp/kiro-version-check-test.json",
        ))
    }

    fn set_version(runtime: &RuntimeConfig, version: &str) {
        let mut config = (*runtime.get()).clone();
        config.kiro_version = version.into();
        runtime.replace(config);
    }

    /// 本机起一个返回固定元数据的 HTTP 服务
    async fn metadata_server(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/meta.json",
            axum::routing::get(move || async move { body }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/meta.json")
    }

    /// 不可达地址，模拟拉取失败
    const UNREACHABLE: &str = "http://127.0.0.1:9/meta.json";

    #[test]
    fn compares_numeric_segments() {
        assert_eq!(compare_versions("0.11.107", "0.11.133"), Ok(Ordering::Less));
        assert_eq!(compare_versions("0.11.133", "1.5.37"), Ok(Ordering::Less));
        assert_eq!(compare_versions("0.11.107", "1.5.37"), Ok(Ordering::Less));
        assert_eq!(compare_versions("1.5.37", "1.5.9"), Ok(Ordering::Greater));
        assert_eq!(
            compare_versions("0.11.133", "0.11.133"),
            Ok(Ordering::Equal)
        );
        assert_eq!(compare_versions("1.5", "1.5.0"), Ok(Ordering::Equal));
        assert_eq!(compare_versions("1.5.0.1", "1.5"), Ok(Ordering::Greater));
        assert_eq!(compare_versions(" 1.5.37 ", "1.5.37"), Ok(Ordering::Equal));
    }

    #[test]
    fn rejects_invalid_versions() {
        for bad in [
            "", "abc", "1..2", "1.5.x", "1.5-beta", "v1.5", "1.5.", "-1.2",
        ] {
            assert!(parse_version(bad).is_err(), "{bad:?} 应解析失败");
            assert!(compare_versions(bad, "1.0").is_err());
        }
        let (status, error) = evaluate("abc", "0.11.133", Some("1.5.37"), false);
        assert_eq!(status, VersionStatus::Unknown);
        assert!(error.unwrap().contains("abc"));
    }

    #[test]
    fn evaluates_status() {
        assert_eq!(
            evaluate("1.5.37", "0.11.133", Some("1.5.37"), false).0,
            VersionStatus::Ok
        );
        assert_eq!(
            evaluate("1.6.0", "0.11.133", Some("1.5.37"), false).0,
            VersionStatus::Ok
        );
        assert_eq!(
            evaluate("0.11.133", "0.11.133", Some("1.5.37"), false).0,
            VersionStatus::BehindLatest
        );
        assert_eq!(
            evaluate("0.11.107", "0.11.133", Some("1.5.37"), false).0,
            VersionStatus::Unsupported
        );
        // 从未拉到最新版：仍可按静态最低版本判断
        assert_eq!(
            evaluate("0.11.107", "0.11.133", None, false).0,
            VersionStatus::Unsupported
        );
        assert_eq!(
            evaluate("0.11.133", "0.11.133", None, false).0,
            VersionStatus::Ok
        );
        // 上游拒绝优先
        assert_eq!(
            evaluate("1.5.37", "0.11.133", Some("1.5.37"), true).0,
            VersionStatus::Unsupported
        );
    }

    #[test]
    fn parses_official_metadata() {
        let (version, pub_date) = parse_metadata(METADATA).unwrap();
        assert_eq!(version, "1.5.37");
        assert_eq!(pub_date.as_deref(), Some("2026-11-24"));
        assert!(parse_metadata(r#"{"currentRelease":"latest"}"#).is_err());
        assert!(parse_metadata("not json").is_err());
    }

    #[tokio::test]
    async fn fetch_failure_still_reports_unsupported() {
        let service = VersionCheckService::with_metadata_url(runtime("0.11.107"), UNREACHABLE);
        assert_eq!(service.state().status, VersionStatus::Unsupported);
        let state = service.check_now().await;
        assert_eq!(state.status, VersionStatus::Unsupported);
        assert!(state.checked_at.is_some());
        assert!(state.latest_version.is_none());
        assert!(state.last_error.unwrap().contains("获取官方最新版本失败"));
    }

    #[tokio::test]
    async fn fetch_success_then_failure_keeps_last_result() {
        let rt = runtime("0.11.133");
        let url = metadata_server(METADATA).await;
        let service = VersionCheckService::with_metadata_url(rt.clone(), url);
        let state = service.check_now().await;
        assert_eq!(state.status, VersionStatus::BehindLatest, "{state:?}");
        assert_eq!(state.latest_version.as_deref(), Some("1.5.37"));
        assert_eq!(state.latest_pub_date.as_deref(), Some("2026-11-24"));
        assert!(state.last_error.is_none());

        // 配置修改后立即反映，无需重新拉取
        set_version(&rt, "1.5.37");
        assert_eq!(service.state().status, VersionStatus::Ok);
        set_version(&rt, "0.11.107");
        assert_eq!(service.state().status, VersionStatus::Unsupported);

        // 拉取失败时保留上次成功结果
        let failing = VersionCheckService::with_metadata_url(rt.clone(), UNREACHABLE);
        *failing.cached.write() = Cached {
            latest_version: Some("1.5.37".into()),
            latest_pub_date: Some("2026-11-24".into()),
            ..Default::default()
        };
        set_version(&rt, "0.11.133");
        let state = failing.check_now().await;
        assert_eq!(state.latest_version.as_deref(), Some("1.5.37"));
        assert_eq!(state.status, VersionStatus::BehindLatest);
        assert!(state.last_error.is_some());
    }

    #[tokio::test]
    async fn manual_refresh_is_throttled() {
        let service = VersionCheckService::with_metadata_url(runtime("0.11.133"), UNREACHABLE);
        let first = service.refresh_manual().await;
        assert!(!first.throttled);
        let checked_at = first.state.checked_at.clone();
        let second = service.refresh_manual().await;
        assert!(second.throttled);
        assert_eq!(second.state.checked_at, checked_at);
    }

    #[tokio::test]
    async fn upstream_rejection_clears_only_after_version_changed() {
        let rt = runtime("1.5.37");
        let url = metadata_server(METADATA).await;
        let service = VersionCheckService::with_metadata_url(rt.clone(), url);
        assert!(!service.note_upstream_rejection(r#"{"message":"Improperly formed request"}"#));
        assert_eq!(service.state().status, VersionStatus::Ok);

        assert!(service.note_upstream_rejection(
            r#"{"message":"This version of Kiro is NO LONGER SUPPORTED. Please update."}"#
        ));
        let state = service.state();
        assert_eq!(state.status, VersionStatus::Unsupported);
        assert!(
            state
                .last_error
                .unwrap()
                .starts_with("上游拒绝：This version of Kiro is no longer supported（")
        );

        // 拉取失败不清除标记
        let failing = VersionCheckService::with_metadata_url(rt.clone(), UNREACHABLE);
        failing.note_upstream_rejection("no longer supported");
        assert_eq!(failing.check_now().await.status, VersionStatus::Unsupported);

        // 被拒绝版本 1.5.37 ≥ 清单最低 0.11.133：提示清单可能过期
        assert!(
            service
                .state()
                .last_error
                .unwrap()
                .contains("官方最低支持版本可能已高于本地清单 0.11.133，请核对官方公告")
        );

        // 版本未改动：成功检测也不清除
        let state = service.check_now().await;
        assert!(state.latest_version.is_some(), "{state:?}");
        assert_eq!(state.status, VersionStatus::Unsupported, "{state:?}");

        // 改成低于最低版本：仍不清除
        set_version(&rt, "0.11.107");
        assert_eq!(service.check_now().await.status, VersionStatus::Unsupported);

        // 改成更高版本后，成功检测清除
        set_version(&rt, "1.6.0");
        let state = service.check_now().await;
        assert_eq!(state.status, VersionStatus::Ok, "{state:?}");
        assert!(state.last_error.is_none(), "{state:?}");
    }

    #[test]
    fn rejection_below_notice_min_has_no_stale_hint() {
        let service = VersionCheckService::with_metadata_url(runtime("0.11.107"), UNREACHABLE);
        service.note_upstream_rejection("This version of Kiro is no longer supported");
        let error = service.state().last_error.unwrap();
        assert!(error.starts_with("上游拒绝"));
        assert!(!error.contains("可能已高于本地清单"));
    }
}
