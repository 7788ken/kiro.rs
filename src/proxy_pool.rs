//! 出口代理池
//!
//! 管理员维护一组出口代理，每个凭据通过 `proxyId` 固定绑定一个代理（即固定出口 IP）。
//! - 代理条目持久化到凭据文件同目录的 `proxies.json`（原子写）
//! - 健康状态持久化到 `proxy_health.json`（允许丢失，重启后重新检测）
//! - 代理密码与 credentials.json 中的 `proxyPassword` 一样明文落盘，但不会出现在接口和日志中

use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::http_client::{ProxyConfig, build_client};
use crate::kiro::model::credentials::KiroCredentials;
use crate::model::config::{Config, TlsBackend};

/// 默认出口 IP 检测地址
pub const DEFAULT_EGRESS_CHECK_URL: &str = "https://checkip.amazonaws.com";
/// 出口检测超时（秒）
const CHECK_TIMEOUT_SECS: u64 = 10;
/// `check_all` 并发度
const CHECK_CONCURRENCY: usize = 4;
/// 非 IP 响应体写入 last_error 时保留的最大字符数
const BODY_PREVIEW_CHARS: usize = 100;
/// 错误信息最大字符数
const ERROR_MAX_CHARS: usize = 300;

/// 代理池条目（持久化到 proxies.json）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyEntry {
    pub id: u64,
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: String,
}

fn default_true() -> bool {
    true
}

impl ProxyEntry {
    /// 转换为 HTTP Client 使用的代理配置
    pub fn to_proxy_config(&self) -> ProxyConfig {
        let mut proxy = ProxyConfig::new(&self.url);
        if let (Some(username), Some(password)) = (&self.username, &self.password) {
            proxy = proxy.with_auth(username, password);
        }
        proxy
    }
}

/// 健康状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyStatus {
    Healthy,
    Unhealthy,
    #[default]
    Unknown,
}

/// 代理健康信息（持久化到 proxy_health.json，允许丢失）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProxyHealth {
    pub status: ProxyStatus,
    #[serde(default)]
    pub egress_ip: Option<String>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub checked_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// 新增/更新代理的输入
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInput {
    pub name: String,
    pub url: String,
    /// 用户名：空字符串表示清空
    #[serde(default)]
    pub username: Option<String>,
    /// 密码：传了非空值就覆盖，传空或不传保持不变
    #[serde(default)]
    pub password: Option<String>,
    /// 为 true 时清空已有密码
    #[serde(default)]
    pub clear_password: bool,
    #[serde(default)]
    pub note: Option<String>,
    /// 仅新增时生效，默认 true
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// 代理池错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyPoolError {
    NotFound(u64),
    Invalid(String),
    /// 批量导入时无法识别的代理行（消息中带脱敏后的原文）
    InvalidUrl(String),
    InUse { id: u64, bound: usize },
    Storage(String),
}

impl fmt::Display for ProxyPoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyPoolError::NotFound(id) => write!(f, "代理不存在: {}", id),
            ProxyPoolError::Invalid(msg) | ProxyPoolError::InvalidUrl(msg) => write!(f, "{}", msg),
            ProxyPoolError::InUse { id, bound } => write!(
                f,
                "代理 #{} 仍绑定 {} 个未归档凭据，请先改绑后再删除",
                id, bound
            ),
            ProxyPoolError::Storage(msg) => write!(f, "保存代理池失败: {}", msg),
        }
    }
}

impl std::error::Error for ProxyPoolError {}

/// 自动分配失败
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickError {
    /// 池中有启用的代理，但没有健康的
    NoHealthyProxy,
}

/// 批量导入中被跳过或失败的行
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchLineIssue {
    /// 原文中的行号（从 1 开始）
    pub line: usize,
    pub reason: String,
}

/// 批量导入结果（`added` 含密码，不要直接序列化给前端）
#[derive(Debug, Default)]
pub struct BatchImportResult {
    pub added: Vec<ProxyEntry>,
    pub skipped: Vec<BatchLineIssue>,
    pub failed: Vec<BatchLineIssue>,
}

/// proxies.json 文件格式
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxiesFile {
    /// 下一个分配的 ID（删除后不复用，避免历史绑定指向新代理）
    #[serde(default)]
    next_id: u64,
    #[serde(default)]
    proxies: Vec<ProxyEntry>,
}

#[derive(Debug, Clone)]
struct CheckOptions {
    egress_check_url: String,
    tls_backend: TlsBackend,
}

struct PoolState {
    entries: Vec<ProxyEntry>,
    next_id: u64,
}

/// 出口代理池
pub struct ProxyPool {
    state: RwLock<PoolState>,
    health: RwLock<HashMap<u64, ProxyHealth>>,
    global_proxy: RwLock<Option<ProxyConfig>>,
    options: RwLock<CheckOptions>,
    proxies_path: Option<PathBuf>,
    health_path: Option<PathBuf>,
    /// 串行化落盘
    write_lock: Mutex<()>,
    /// 测试用：proxies.json 写入次数
    #[cfg(test)]
    entries_persist_count: std::sync::atomic::AtomicUsize,
}

impl ProxyPool {
    /// 无文件的内存代理池（测试或无凭据文件路径时使用）
    pub fn in_memory(global_proxy: Option<ProxyConfig>) -> Self {
        Self {
            state: RwLock::new(PoolState {
                entries: Vec::new(),
                next_id: 1,
            }),
            health: RwLock::new(HashMap::new()),
            global_proxy: RwLock::new(global_proxy),
            options: RwLock::new(CheckOptions {
                egress_check_url: DEFAULT_EGRESS_CHECK_URL.to_string(),
                tls_backend: TlsBackend::default(),
            }),
            proxies_path: None,
            health_path: None,
            write_lock: Mutex::new(()),
            #[cfg(test)]
            entries_persist_count: Default::default(),
        }
    }

    /// 从目录加载 `proxies.json` 与 `proxy_health.json`
    ///
    /// proxies.json 解析失败时返回错误（避免启动后覆盖掉管理员维护的数据）；
    /// proxy_health.json 解析失败只记录警告。
    pub fn load(
        dir: &Path,
        global_proxy: Option<ProxyConfig>,
        config: &Config,
    ) -> anyhow::Result<Self> {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        let proxies_path = dir.join("proxies.json");
        let health_path = dir.join("proxy_health.json");

        let file: ProxiesFile = match std::fs::read_to_string(&proxies_path) {
            Ok(content) if !content.trim().is_empty() => serde_json::from_str(&content)
                .map_err(|e| anyhow::anyhow!("解析 {} 失败: {}", proxies_path.display(), e))?,
            Ok(_) => ProxiesFile::default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProxiesFile::default(),
            Err(e) => anyhow::bail!("读取 {} 失败: {}", proxies_path.display(), e),
        };
        let max_id = file.proxies.iter().map(|p| p.id).max().unwrap_or(0);
        let next_id = file.next_id.max(max_id + 1);

        let health: HashMap<u64, ProxyHealth> = match std::fs::read_to_string(&health_path) {
            Ok(content) => match serde_json::from_str::<HashMap<String, ProxyHealth>>(&content) {
                Ok(map) => map
                    .into_iter()
                    .filter_map(|(k, v)| k.parse::<u64>().ok().map(|id| (id, v)))
                    .collect(),
                Err(e) => {
                    tracing::warn!("解析代理健康缓存失败，将忽略: {}", e);
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(),
        };

        tracing::info!("已加载 {} 个出口代理", file.proxies.len());

        let pool = Self {
            state: RwLock::new(PoolState {
                entries: file.proxies,
                next_id,
            }),
            health: RwLock::new(health),
            global_proxy: RwLock::new(global_proxy),
            options: RwLock::new(CheckOptions {
                egress_check_url: config.effective_egress_check_url().to_string(),
                tls_backend: config.tls_backend,
            }),
            proxies_path: Some(proxies_path),
            health_path: Some(health_path),
            write_lock: Mutex::new(()),
            #[cfg(test)]
            entries_persist_count: Default::default(),
        };
        Ok(pool)
    }

    /// 更新全局代理（热重载时调用）
    pub fn set_global_proxy(&self, proxy: Option<ProxyConfig>) {
        *self.global_proxy.write() = proxy;
    }

    /// 更新出口检测选项（热重载时调用）
    pub fn apply_config(&self, config: &Config) {
        *self.options.write() = CheckOptions {
            egress_check_url: config.effective_egress_check_url().to_string(),
            tls_backend: config.tls_backend,
        };
    }

    /// 当前全局代理
    pub fn global_proxy(&self) -> Option<ProxyConfig> {
        self.global_proxy.read().clone()
    }

    // ============ 查询 ============

    /// 列出所有代理及其健康状态（按 ID 升序）
    pub fn list(&self) -> Vec<(ProxyEntry, ProxyHealth)> {
        let state = self.state.read();
        let health = self.health.read();
        let mut items: Vec<_> = state
            .entries
            .iter()
            .map(|e| (e.clone(), health.get(&e.id).cloned().unwrap_or_default()))
            .collect();
        items.sort_by_key(|(e, _)| e.id);
        items
    }

    pub fn get(&self, id: u64) -> Option<ProxyEntry> {
        self.state.read().entries.iter().find(|e| e.id == id).cloned()
    }

    pub fn health_of(&self, id: u64) -> ProxyHealth {
        self.health.read().get(&id).cloned().unwrap_or_default()
    }

    // ============ 增删改 ============

    pub fn add(&self, input: ProxyInput) -> Result<ProxyEntry, ProxyPoolError> {
        let entry = {
            let mut state = self.state.write();
            let entry = build_new_entry(input, state.next_id)?;
            state.next_id += 1;
            state.entries.push(entry.clone());
            entry
        };
        if let Err(e) = self.persist_entries() {
            // 回滚内存状态
            self.state.write().entries.retain(|p| p.id != entry.id);
            return Err(e);
        }
        tracing::info!("已添加出口代理 #{} ({})", entry.id, entry.name);
        Ok(entry)
    }

    /// 批量新增代理，成功的条目全部加入后只落盘一次
    ///
    /// `inputs` 中的 usize 为原文行号。校验同 [`ProxyPool::add`]；
    /// URL（scheme/host 忽略大小写）+ 用户名与池中已有条目或本批前面的条目相同则跳过。
    /// 落盘失败时整批回滚，全部记为失败。
    pub fn add_batch(&self, inputs: Vec<(usize, ProxyInput)>) -> BatchImportResult {
        let mut result = BatchImportResult::default();
        let mut added_lines = Vec::new();
        {
            let mut state = self.state.write();
            let mut seen: HashMap<(String, Option<String>), String> = state
                .entries
                .iter()
                .map(|e| (dedup_key(&e.url, &e.username), format!("已存在 #{}", e.id)))
                .collect();
            for (line, input) in inputs {
                let entry = match build_new_entry(input, state.next_id) {
                    Ok(entry) => entry,
                    Err(e) => {
                        result.failed.push(BatchLineIssue {
                            line,
                            reason: e.to_string(),
                        });
                        continue;
                    }
                };
                let key = dedup_key(&entry.url, &entry.username);
                if let Some(reason) = seen.get(&key) {
                    result.skipped.push(BatchLineIssue {
                        line,
                        reason: reason.clone(),
                    });
                    continue;
                }
                seen.insert(key, format!("与第 {} 行重复", line));
                state.next_id += 1;
                state.entries.push(entry.clone());
                result.added.push(entry);
                added_lines.push(line);
            }
        }
        if result.added.is_empty() {
            return result;
        }
        if let Err(e) = self.persist_entries() {
            let added_ids: Vec<u64> = result.added.iter().map(|e| e.id).collect();
            self.state
                .write()
                .entries
                .retain(|p| !added_ids.contains(&p.id));
            let reason = e.to_string();
            result.added.clear();
            result.failed.extend(added_lines.into_iter().map(|line| BatchLineIssue {
                line,
                reason: reason.clone(),
            }));
            result.failed.sort_by_key(|issue| issue.line);
            return result;
        }
        tracing::info!("已批量添加 {} 个出口代理", result.added.len());
        result
    }

    pub fn update(&self, id: u64, input: ProxyInput) -> Result<ProxyEntry, ProxyPoolError> {
        let name = normalize_name(&input.name)?;
        let url = validate_proxy_url(&input.url)?;
        let (previous, updated) = {
            let mut state = self.state.write();
            let entry = state
                .entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(ProxyPoolError::NotFound(id))?;
            let previous = entry.clone();
            let mut next = entry.clone();
            next.name = name;
            next.url = url;
            next.username = normalize_opt(input.username);
            if input.clear_password {
                next.password = None;
            } else if let Some(password) = normalize_opt(input.password) {
                next.password = Some(password);
            }
            next.note = normalize_opt(input.note);
            validate_auth_pair(&next.username, &next.password)?;
            *entry = next.clone();
            (previous, next)
        };
        if let Err(e) = self.persist_entries() {
            if let Some(entry) = self.state.write().entries.iter_mut().find(|e| e.id == id) {
                *entry = previous;
            }
            return Err(e);
        }
        // 地址或认证变化后旧的出口 IP 不再可信，重置为未知
        if previous.to_proxy_config() != updated.to_proxy_config() {
            self.health.write().remove(&id);
            self.persist_health();
        }
        tracing::info!("已更新出口代理 #{} ({})", id, updated.name);
        Ok(updated)
    }

    /// 删除代理；`bound_count` 为当前绑定该代理的未归档凭据数，大于 0 时拒绝
    pub fn remove(&self, id: u64, bound_count: usize) -> Result<(), ProxyPoolError> {
        if bound_count > 0 {
            return Err(ProxyPoolError::InUse {
                id,
                bound: bound_count,
            });
        }
        let removed = {
            let mut state = self.state.write();
            let index = state
                .entries
                .iter()
                .position(|e| e.id == id)
                .ok_or(ProxyPoolError::NotFound(id))?;
            (index, state.entries.remove(index))
        };
        if let Err(e) = self.persist_entries() {
            self.state.write().entries.insert(removed.0, removed.1);
            return Err(e);
        }
        self.health.write().remove(&id);
        self.persist_health();
        tracing::info!("已删除出口代理 #{}", id);
        Ok(())
    }

    pub fn set_enabled(&self, id: u64, enabled: bool) -> Result<ProxyEntry, ProxyPoolError> {
        let (previous, updated) = {
            let mut state = self.state.write();
            let entry = state
                .entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(ProxyPoolError::NotFound(id))?;
            let previous = entry.enabled;
            entry.enabled = enabled;
            (previous, entry.clone())
        };
        if let Err(e) = self.persist_entries() {
            if let Some(entry) = self.state.write().entries.iter_mut().find(|e| e.id == id) {
                entry.enabled = previous;
            }
            return Err(e);
        }
        Ok(updated)
    }

    // ============ 解析与分配 ============

    /// 解析凭据实际使用的代理
    ///
    /// 优先级：
    /// 1. `proxyUrl == "direct"` → 不使用代理
    /// 2. `proxyId` 存在于池中 → 该代理（不管健康状态，不回退，避免换出口 IP）
    /// 3. `proxyId` 指向不存在的条目 → 记 warn，按无 proxyId 处理
    /// 4. 凭据直填代理 > 全局代理 > 无代理
    pub fn resolve(&self, credentials: &KiroCredentials) -> Option<ProxyConfig> {
        if credentials
            .proxy_url
            .as_deref()
            .is_some_and(|url| url.eq_ignore_ascii_case(KiroCredentials::PROXY_DIRECT))
        {
            return None;
        }
        if let Some(proxy_id) = credentials.proxy_id {
            if let Some(entry) = self.get(proxy_id) {
                return Some(entry.to_proxy_config());
            }
            tracing::warn!(
                "凭据 #{:?} 绑定的出口代理 #{} 不存在，回退到凭据直填代理或全局代理",
                credentials.id,
                proxy_id
            );
        }
        let global = self.global_proxy.read();
        credentials.effective_proxy(global.as_ref())
    }

    /// 为新凭据挑选代理
    ///
    /// - 池中没有启用的代理 → `Ok(None)`（不使用代理池，保持老行为）
    /// - 有启用的代理但没有健康的 → `Err(NoHealthyProxy)`
    /// - 否则在启用且健康的代理中按 `(绑定数, id)` 升序取第一个
    pub fn pick_for_new_credential(
        &self,
        bound_counts: &HashMap<u64, usize>,
    ) -> Result<Option<u64>, PickError> {
        let state = self.state.read();
        let health = self.health.read();
        let enabled: Vec<&ProxyEntry> = state.entries.iter().filter(|e| e.enabled).collect();
        if enabled.is_empty() {
            return Ok(None);
        }
        enabled
            .into_iter()
            .filter(|e| {
                health
                    .get(&e.id)
                    .is_some_and(|h| h.status == ProxyStatus::Healthy)
            })
            .map(|e| (bound_counts.get(&e.id).copied().unwrap_or(0), e.id))
            .min()
            .map(|(_, id)| Some(id))
            .ok_or(PickError::NoHealthyProxy)
    }

    // ============ 出口检测 ============

    /// 检测指定代理的出口 IP，更新健康状态并落盘
    pub async fn check(&self, id: u64) -> Result<ProxyHealth, ProxyPoolError> {
        let entry = self.get(id).ok_or(ProxyPoolError::NotFound(id))?;
        let proxy = entry.to_proxy_config();
        let health = self.check_with(Some(&proxy)).await;
        match health.status {
            ProxyStatus::Healthy => tracing::info!(
                "出口代理 #{} ({}) 健康，出口 IP {}，耗时 {}ms",
                id,
                entry.name,
                health.egress_ip.as_deref().unwrap_or("-"),
                health.latency_ms.unwrap_or(0)
            ),
            _ => tracing::warn!(
                "出口代理 #{} ({}) 检测失败: {}",
                id,
                entry.name,
                health.last_error.as_deref().unwrap_or("-")
            ),
        }
        // 检测期间条目可能已被删除
        if self.get(id).is_some() {
            self.health.write().insert(id, health.clone());
            self.persist_health();
        }
        Ok(health)
    }

    /// 并发检测所有启用的代理（并发度 4）
    pub async fn check_all(&self) -> Vec<(u64, ProxyHealth)> {
        let ids: Vec<u64> = self
            .state
            .read()
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.id)
            .collect();
        self.check_many(ids).await
    }

    /// 并发检测指定代理（并发度 4），不存在的 ID 被忽略
    pub async fn check_many(&self, ids: Vec<u64>) -> Vec<(u64, ProxyHealth)> {
        futures::stream::iter(ids)
            .map(|id| async move { (id, self.check(id).await) })
            .buffer_unordered(CHECK_CONCURRENCY)
            .filter_map(|(id, result)| async move { result.ok().map(|h| (id, h)) })
            .collect()
            .await
    }

    /// 用给定代理（None 表示直连）做一次出口检测，不写入池健康状态
    pub async fn check_with(&self, proxy: Option<&ProxyConfig>) -> ProxyHealth {
        let options = self.options.read().clone();
        let started = Instant::now();
        let checked_at = Some(Utc::now().to_rfc3339());
        let result: anyhow::Result<(bool, String, String)> = async {
            let client = build_client(proxy, CHECK_TIMEOUT_SECS, options.tls_backend)?;
            let response = client.get(&options.egress_check_url).send().await?;
            let status = response.status();
            let body = response.text().await?;
            Ok((status.is_success(), status.to_string(), body))
        }
        .await;
        let latency_ms = Some(started.elapsed().as_millis() as u64);
        match result {
            Ok((ok, status, body)) => {
                let mut health = evaluate_egress_response(ok, &status, &body);
                health.latency_ms = latency_ms;
                health.checked_at = checked_at;
                health
            }
            Err(e) => ProxyHealth {
                status: ProxyStatus::Unhealthy,
                egress_ip: None,
                latency_ms,
                checked_at,
                last_error: Some(truncate_chars(&format!("{:#}", e), ERROR_MAX_CHARS)),
            },
        }
    }

    // ============ 持久化 ============

    fn persist_entries(&self) -> Result<(), ProxyPoolError> {
        let Some(path) = &self.proxies_path else {
            return Ok(());
        };
        let _guard = self.write_lock.lock();
        #[cfg(test)]
        self.entries_persist_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let json = {
            let state = self.state.read();
            let mut proxies = state.entries.clone();
            proxies.sort_by_key(|p| p.id);
            serde_json::to_string_pretty(&ProxiesFile {
                next_id: state.next_id,
                proxies,
            })
            .map_err(|e| ProxyPoolError::Storage(e.to_string()))?
        };
        run_blocking(|| -> anyhow::Result<()> {
            let _lock = crate::shared_files::acquire(&path.with_extension("lock"))?;
            crate::shared_files::atomic_write(path, &json)
        })
        .map_err(|e| ProxyPoolError::Storage(e.to_string()))
    }

    fn persist_health(&self) {
        let Some(path) = &self.health_path else {
            return;
        };
        let _guard = self.write_lock.lock();
        let json = {
            let health = self.health.read();
            let map: HashMap<String, &ProxyHealth> =
                health.iter().map(|(k, v)| (k.to_string(), v)).collect();
            match serde_json::to_string_pretty(&map) {
                Ok(json) => json,
                Err(e) => {
                    tracing::warn!("序列化代理健康缓存失败: {}", e);
                    return;
                }
            }
        };
        if let Err(e) = run_blocking(|| crate::shared_files::atomic_write(path, &json)) {
            tracing::warn!("保存代理健康缓存失败: {}", e);
        }
    }

    /// 测试辅助：直接设置健康状态
    #[cfg(test)]
    pub fn set_health_for_test(&self, id: u64, health: ProxyHealth) {
        self.health.write().insert(id, health);
    }

    /// 测试辅助：设置检测地址
    #[cfg(test)]
    pub fn set_egress_check_url_for_test(&self, url: &str) {
        self.options.write().egress_check_url = url.to_string();
    }
}

/// 在多线程 Tokio runtime 内用 block_in_place 执行同步 IO，其他情况直接执行
fn run_blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// 根据出口检测的响应判断健康状态（不含耗时和时间）
pub fn evaluate_egress_response(success: bool, status: &str, body: &str) -> ProxyHealth {
    let trimmed = body.trim();
    if !success {
        return ProxyHealth {
            status: ProxyStatus::Unhealthy,
            last_error: Some(format!(
                "HTTP {}: {}",
                status,
                truncate_chars(trimmed, BODY_PREVIEW_CHARS)
            )),
            ..Default::default()
        };
    }
    match trimmed.parse::<IpAddr>() {
        Ok(ip) => ProxyHealth {
            status: ProxyStatus::Healthy,
            egress_ip: Some(ip.to_string()),
            ..Default::default()
        },
        Err(_) => ProxyHealth {
            status: ProxyStatus::Unhealthy,
            last_error: Some(format!(
                "响应不是 IP 地址: {}",
                truncate_chars(trimmed, BODY_PREVIEW_CHARS)
            )),
            ..Default::default()
        },
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn normalize_opt(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn normalize_name(name: &str) -> Result<String, ProxyPoolError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ProxyPoolError::Invalid("代理名称不能为空".into()));
    }
    if name.chars().count() > 64 {
        return Err(ProxyPoolError::Invalid("代理名称不能超过 64 个字符".into()));
    }
    Ok(name.to_string())
}

/// 校验输入并构造新条目（不写入池、不落盘）
fn build_new_entry(input: ProxyInput, id: u64) -> Result<ProxyEntry, ProxyPoolError> {
    let name = normalize_name(&input.name)?;
    let url = validate_proxy_url(&input.url)?;
    let username = normalize_opt(input.username);
    let password = if input.clear_password {
        None
    } else {
        normalize_opt(input.password)
    };
    validate_auth_pair(&username, &password)?;
    Ok(ProxyEntry {
        id,
        name,
        url,
        username,
        password,
        note: normalize_opt(input.note),
        enabled: input.enabled.unwrap_or(true),
        created_at: Utc::now().to_rfc3339(),
    })
}

/// 去重键：`scheme://host:port`（scheme/host 小写，端口缺省时取协议默认端口）+ 用户名
fn dedup_key(url: &str, username: &Option<String>) -> (String, Option<String>) {
    let normalized = match reqwest::Url::parse(url.trim()) {
        Ok(parsed) => format!(
            "{}://{}:{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or("").to_ascii_lowercase(),
            parsed
                .port_or_known_default()
                .map(|p| p.to_string())
                .unwrap_or_default(),
            parsed.path().trim_end_matches('/')
        ),
        Err(_) => url.trim().to_ascii_lowercase(),
    };
    (normalized, username.clone())
}

/// 批量导入行中无协议前缀时允许的默认协议
pub const IMPORT_SCHEMES: [&str; 4] = ["http", "https", "socks5", "socks5h"];
/// 无法识别的行在错误信息中保留的最大字符数
const LINE_PREVIEW_CHARS: usize = 80;

/// 解析批量导入中的一行代理
///
/// 支持：
/// - `scheme://user:pass@host:port`（user/pass 会做 percent-decode）
/// - `scheme://host:port`
/// - `host:port:user:pass` / `host:port`（使用 `default_scheme`）
///
/// host 可为 IPv4、`[IPv6]` 或域名。名称自动生成为 `host:port`。
/// 空行和 `#` 注释行由调用方跳过。
pub fn parse_proxy_line(line: &str, default_scheme: &str) -> Result<ProxyInput, ProxyPoolError> {
    let line = line.trim();
    let invalid = || {
        ProxyPoolError::InvalidUrl(format!(
            "无法识别的代理格式: {}",
            truncate_chars(&redact_line(line), LINE_PREVIEW_CHARS)
        ))
    };
    let default_scheme = default_scheme.trim().to_ascii_lowercase();
    if !IMPORT_SCHEMES.contains(&default_scheme.as_str()) {
        return Err(ProxyPoolError::Invalid(
            "默认协议必须是 http、https、socks5 或 socks5h".into(),
        ));
    }
    if line.is_empty() || line.chars().any(char::is_whitespace) {
        return Err(invalid());
    }

    let (scheme, host, port, username, password) = if let Some(index) = line.find("://") {
        let parsed = reqwest::Url::parse(line).map_err(|_| invalid())?;
        if !IMPORT_SCHEMES.contains(&parsed.scheme()) {
            return Err(invalid());
        }
        // 只接受 [userinfo@]host:port，不允许路径、查询等；必须显式写端口
        let rest = &line[index + 3..];
        let authority = rest
            .rsplit_once('@')
            .map_or(rest, |(_, host_port)| host_port)
            .trim_end_matches('/');
        let explicit_port = authority.rsplit_once(':').is_some_and(|(host, port)| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && (!host.starts_with('[') || host.ends_with(']'))
        });
        if !matches!(parsed.path(), "" | "/")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !explicit_port
        {
            return Err(invalid());
        }
        let host = parsed.host_str().filter(|h| !h.is_empty()).ok_or_else(invalid)?;
        let port = parsed.port_or_known_default().ok_or_else(invalid)?;
        let username = percent_decode(parsed.username()).ok_or_else(invalid)?;
        let password = match parsed.password() {
            Some(p) => percent_decode(p).ok_or_else(invalid)?,
            None => String::new(),
        };
        (
            parsed.scheme().to_string(),
            host.to_string(),
            port,
            username,
            password,
        )
    } else {
        let (host, rest) = split_host(line).ok_or_else(invalid)?;
        let mut parts = rest.splitn(3, ':');
        let port = parts.next().ok_or_else(invalid)?;
        let username = parts.next();
        let password = parts.next();
        let (username, password) = match (username, password) {
            (None, None) => (String::new(), String::new()),
            (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => (u.to_string(), p.to_string()),
            _ => return Err(invalid()),
        };
        let port: u16 = port.parse().map_err(|_| invalid())?;
        (default_scheme, host.to_string(), port, username, password)
    };
    if port == 0 {
        return Err(invalid());
    }
    let host_port = format!("{}:{}", host, port);
    Ok(ProxyInput {
        name: truncate_chars(&host_port, 64),
        url: format!("{}://{}", scheme, host_port),
        username: Some(username).filter(|v| !v.is_empty()),
        password: Some(password).filter(|v| !v.is_empty()),
        ..Default::default()
    })
}

/// 拆出无协议行的 host 与剩余部分（`host:rest`），IPv6 需用方括号包裹
fn split_host(line: &str) -> Option<(&str, &str)> {
    if line.starts_with('[') {
        let end = line.find(']')?;
        let host = &line[..=end];
        host[1..end].parse::<std::net::Ipv6Addr>().ok()?;
        let rest = line[end + 1..].strip_prefix(':')?;
        Some((host, rest))
    } else {
        let (host, rest) = line.split_once(':')?;
        if host.is_empty() || host.contains(['[', ']', '@', '/']) {
            return None;
        }
        Some((host, rest))
    }
}

fn percent_decode(value: &str) -> Option<String> {
    urlencoding::decode(value).ok().map(|v| v.into_owned())
}

/// 错误信息里的原文去掉可能的密码：`user:pass@` → `user:***@`；`host:port:user:pass` → 末段打码
fn redact_line(line: &str) -> String {
    if let Some(index) = line.find("://") {
        let rest = &line[index + 3..];
        if let Some(at) = rest.rfind('@') {
            let userinfo = &rest[..at];
            let user = userinfo.split(':').next().unwrap_or("");
            return format!("{}{}:***@{}", &line[..index + 3], user, &rest[at + 1..]);
        }
        return line.to_string();
    }
    // host:port:user:pass → 保留 host:port:user，其余打码
    let host_end = if line.starts_with('[') {
        line.find(']').map_or(0, |i| i + 1)
    } else {
        line.find(':').unwrap_or(line.len())
    };
    let (host, rest) = line.split_at(host_end);
    let parts: Vec<&str> = rest.trim_start_matches(':').splitn(3, ':').collect();
    if parts.len() == 3 {
        return format!("{}:{}:{}:***", host, parts[0], parts[1]);
    }
    line.to_string()
}

fn validate_auth_pair(
    username: &Option<String>,
    password: &Option<String>,
) -> Result<(), ProxyPoolError> {
    if username.is_some() != password.is_some() {
        return Err(ProxyPoolError::Invalid(
            "代理用户名和密码需要同时填写，或都留空".into(),
        ));
    }
    Ok(())
}

/// 校验代理 URL：scheme 必须是 http/https/socks5/socks5h，且不能内嵌认证信息
///
/// 使用 reqwest 重导出的 `url::Url` 解析，不引入新依赖。
pub fn validate_proxy_url(url: &str) -> Result<String, ProxyPoolError> {
    let url = url.trim();
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ProxyPoolError::Invalid(
            "代理 URL 格式无效，示例: http://host:port 或 socks5://host:port".into(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return Err(ProxyPoolError::Invalid(
            "代理 URL 协议必须是 http、https、socks5 或 socks5h".into(),
        ));
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(ProxyPoolError::Invalid("代理 URL 缺少主机地址".into()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ProxyPoolError::Invalid(
            "代理 URL 中不能包含用户名或密码，请填写在单独的用户名/密码字段".into(),
        ));
    }
    if reqwest::Proxy::all(url).is_err() {
        return Err(ProxyPoolError::Invalid("代理 URL 无法解析".into()));
    }
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, url: &str) -> ProxyInput {
        ProxyInput {
            name: name.into(),
            url: url.into(),
            ..Default::default()
        }
    }

    fn healthy(ip: &str) -> ProxyHealth {
        ProxyHealth {
            status: ProxyStatus::Healthy,
            egress_ip: Some(ip.into()),
            ..Default::default()
        }
    }

    fn unhealthy() -> ProxyHealth {
        ProxyHealth {
            status: ProxyStatus::Unhealthy,
            last_error: Some("boom".into()),
            ..Default::default()
        }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kiro-proxy-pool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_direct_wins_over_proxy_id_and_global() {
        let pool = ProxyPool::in_memory(Some(ProxyConfig::new("http://global:1")));
        let entry = pool.add(input("a", "http://pool-a:1")).unwrap();
        let creds = KiroCredentials {
            proxy_url: Some("DIRECT".into()),
            proxy_id: Some(entry.id),
            ..Default::default()
        };
        assert_eq!(pool.resolve(&creds), None);
    }

    #[test]
    fn resolve_bound_proxy_even_when_unhealthy() {
        let pool = ProxyPool::in_memory(Some(ProxyConfig::new("http://global:1")));
        let mut add = input("a", "socks5://pool-a:1080");
        add.username = Some("u".into());
        add.password = Some("p".into());
        let entry = pool.add(add).unwrap();
        pool.set_health_for_test(entry.id, unhealthy());
        let creds = KiroCredentials {
            proxy_id: Some(entry.id),
            ..Default::default()
        };
        assert_eq!(
            pool.resolve(&creds),
            Some(ProxyConfig::new("socks5://pool-a:1080").with_auth("u", "p"))
        );
        // 停用的条目同样保持绑定，不回退
        pool.set_enabled(entry.id, false).unwrap();
        assert_eq!(
            pool.resolve(&creds).map(|p| p.url),
            Some("socks5://pool-a:1080".to_string())
        );
    }

    #[test]
    fn resolve_missing_proxy_id_falls_back_to_credential_then_global() {
        let pool = ProxyPool::in_memory(Some(ProxyConfig::new("http://global:1")));
        let with_url = KiroCredentials {
            proxy_id: Some(42),
            proxy_url: Some("http://cred:2".into()),
            ..Default::default()
        };
        assert_eq!(pool.resolve(&with_url), Some(ProxyConfig::new("http://cred:2")));
        let without_url = KiroCredentials {
            proxy_id: Some(42),
            ..Default::default()
        };
        assert_eq!(
            pool.resolve(&without_url),
            Some(ProxyConfig::new("http://global:1"))
        );
    }

    #[test]
    fn resolve_without_proxy_id_keeps_legacy_behavior() {
        let pool = ProxyPool::in_memory(Some(ProxyConfig::new("http://global:1")));
        pool.add(input("a", "http://pool-a:1")).unwrap();
        let creds = KiroCredentials::default();
        assert_eq!(pool.resolve(&creds), Some(ProxyConfig::new("http://global:1")));
        let direct_url = KiroCredentials {
            proxy_url: Some("http://cred:2".into()),
            proxy_username: Some("u".into()),
            proxy_password: Some("p".into()),
            ..Default::default()
        };
        assert_eq!(
            pool.resolve(&direct_url),
            Some(ProxyConfig::new("http://cred:2").with_auth("u", "p"))
        );
        // 热重载更新全局代理
        pool.set_global_proxy(None);
        assert_eq!(pool.resolve(&creds), None);
    }

    #[test]
    fn pick_returns_none_when_no_enabled_proxy() {
        let pool = ProxyPool::in_memory(None);
        assert_eq!(pool.pick_for_new_credential(&HashMap::new()), Ok(None));
        let entry = pool.add(input("a", "http://a:1")).unwrap();
        pool.set_health_for_test(entry.id, healthy("1.1.1.1"));
        pool.set_enabled(entry.id, false).unwrap();
        assert_eq!(pool.pick_for_new_credential(&HashMap::new()), Ok(None));
    }

    #[test]
    fn pick_errors_when_enabled_but_none_healthy() {
        let pool = ProxyPool::in_memory(None);
        let a = pool.add(input("a", "http://a:1")).unwrap();
        pool.add(input("b", "http://b:1")).unwrap(); // Unknown
        pool.set_health_for_test(a.id, unhealthy());
        // 停用但健康的条目不参与
        let c = pool.add(input("c", "http://c:1")).unwrap();
        pool.set_health_for_test(c.id, healthy("3.3.3.3"));
        pool.set_enabled(c.id, false).unwrap();
        assert_eq!(
            pool.pick_for_new_credential(&HashMap::new()),
            Err(PickError::NoHealthyProxy)
        );
    }

    #[test]
    fn pick_prefers_fewest_bound_then_smallest_id() {
        let pool = ProxyPool::in_memory(None);
        let a = pool.add(input("a", "http://a:1")).unwrap();
        let b = pool.add(input("b", "http://b:1")).unwrap();
        let c = pool.add(input("c", "http://c:1")).unwrap();
        let d = pool.add(input("d", "http://d:1")).unwrap();
        for id in [a.id, b.id, c.id] {
            pool.set_health_for_test(id, healthy("1.1.1.1"));
        }
        pool.set_health_for_test(d.id, unhealthy());

        // a=2, b=1, c=1, d=0（不健康）→ b（同数取小 id）
        let counts = HashMap::from([(a.id, 2), (b.id, 1), (c.id, 1)]);
        assert_eq!(pool.pick_for_new_credential(&counts), Ok(Some(b.id)));
        // 全部为 0 → a
        assert_eq!(pool.pick_for_new_credential(&HashMap::new()), Ok(Some(a.id)));
        // c 最少
        let counts = HashMap::from([(a.id, 3), (b.id, 3), (c.id, 2)]);
        assert_eq!(pool.pick_for_new_credential(&counts), Ok(Some(c.id)));
    }

    #[test]
    fn egress_response_must_be_ip() {
        let ok = evaluate_egress_response(true, "200 OK", " 203.0.113.7\n");
        assert_eq!(ok.status, ProxyStatus::Healthy);
        assert_eq!(ok.egress_ip.as_deref(), Some("203.0.113.7"));
        let v6 = evaluate_egress_response(true, "200 OK", "2001:db8::1\n");
        assert_eq!(v6.status, ProxyStatus::Healthy);

        let long_body = format!("<html>{}</html>", "x".repeat(500));
        let bad = evaluate_egress_response(true, "200 OK", &long_body);
        assert_eq!(bad.status, ProxyStatus::Unhealthy);
        assert_eq!(bad.egress_ip, None);
        let error = bad.last_error.unwrap();
        assert!(error.starts_with("响应不是 IP 地址: <html>"), "{error}");
        assert_eq!(
            error.trim_start_matches("响应不是 IP 地址: ").chars().count(),
            BODY_PREVIEW_CHARS
        );

        let http_error = evaluate_egress_response(false, "502 Bad Gateway", "1.2.3.4");
        assert_eq!(http_error.status, ProxyStatus::Unhealthy);
        assert_eq!(http_error.egress_ip, None);
    }

    /// 本地起一个假代理：无论请求什么都返回固定响应体
    async fn fake_proxy(body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn check_marks_non_ip_body_unhealthy_and_ip_body_healthy() {
        let pool = ProxyPool::in_memory(None);
        // 目标用 http，请求会以绝对 URI 形式发给假代理，不会真的出网
        pool.set_egress_check_url_for_test("http://egress.invalid/");
        let bad = pool
            .add(input("bad", &fake_proxy("<html>blocked</html>").await))
            .unwrap();
        let good = pool.add(input("good", &fake_proxy("198.51.100.9\n").await)).unwrap();

        let health = pool.check(bad.id).await.unwrap();
        assert_eq!(health.status, ProxyStatus::Unhealthy);
        assert!(
            health.last_error.as_deref().unwrap().contains("<html>blocked</html>"),
            "{health:?}"
        );
        assert!(health.checked_at.is_some());
        assert!(health.latency_ms.is_some());
        assert_eq!(pool.health_of(bad.id).status, ProxyStatus::Unhealthy);

        let results = pool.check_all().await;
        assert_eq!(results.len(), 2);
        let good_health = pool.health_of(good.id);
        assert_eq!(good_health.status, ProxyStatus::Healthy);
        assert_eq!(good_health.egress_ip.as_deref(), Some("198.51.100.9"));
    }

    #[test]
    fn proxies_json_round_trips_with_atomic_write() {
        let dir = temp_dir();
        let config = Config::default();
        let pool = ProxyPool::load(&dir, None, &config).unwrap();
        let mut add = input(" 东京-1 ", " http://10.0.0.1:3128 ");
        add.username = Some("user".into());
        add.password = Some("secret".into());
        add.note = Some("备注".into());
        let a = pool.add(add).unwrap();
        let b = pool.add(input("b", "socks5h://10.0.0.2:1080")).unwrap();
        pool.set_enabled(b.id, false).unwrap();
        pool.set_health_for_test(a.id, healthy("203.0.113.1"));
        pool.persist_health();

        // 目录下没有残留临时文件
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());

        let reloaded = ProxyPool::load(&dir, None, &config).unwrap();
        let list = reloaded.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, pool.get(a.id).unwrap());
        assert_eq!(list[0].0.name, "东京-1");
        assert_eq!(list[0].0.url, "http://10.0.0.1:3128");
        assert_eq!(list[0].0.password.as_deref(), Some("secret"));
        assert_eq!(list[0].1.egress_ip.as_deref(), Some("203.0.113.1"));
        assert!(!list[1].0.enabled);

        // 删除后 ID 不复用
        reloaded.remove(b.id, 0).unwrap();
        let c = reloaded.add(input("c", "http://10.0.0.3:1")).unwrap();
        assert_eq!(c.id, b.id + 1);
        let again = ProxyPool::load(&dir, None, &config).unwrap();
        assert_eq!(
            again.list().iter().map(|(e, _)| e.id).collect::<Vec<_>>(),
            vec![a.id, c.id]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_proxies_json_is_an_error_but_corrupt_health_is_ignored() {
        let dir = temp_dir();
        std::fs::write(dir.join("proxy_health.json"), "not json").unwrap();
        assert!(ProxyPool::load(&dir, None, &Config::default()).is_ok());
        std::fs::write(dir.join("proxies.json"), "not json").unwrap();
        assert!(ProxyPool::load(&dir, None, &Config::default()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_rejects_bound_proxy() {
        let pool = ProxyPool::in_memory(None);
        let entry = pool.add(input("a", "http://a:1")).unwrap();
        let error = pool.remove(entry.id, 3).unwrap_err();
        assert_eq!(error, ProxyPoolError::InUse { id: entry.id, bound: 3 });
        assert!(error.to_string().contains('3'));
        assert!(pool.get(entry.id).is_some());
        pool.remove(entry.id, 0).unwrap();
        assert!(pool.get(entry.id).is_none());
        assert_eq!(pool.remove(entry.id, 0), Err(ProxyPoolError::NotFound(entry.id)));
    }

    #[test]
    fn update_password_semantics() {
        let pool = ProxyPool::in_memory(None);
        let mut add = input("a", "http://a:1");
        add.username = Some("u".into());
        add.password = Some("p1".into());
        let entry = pool.add(add).unwrap();
        pool.set_health_for_test(entry.id, healthy("1.1.1.1"));

        // 密码留空保持不变；只改名称不重置健康状态
        let mut keep = input("renamed", "http://a:1");
        keep.username = Some("u".into());
        keep.password = Some(String::new());
        let updated = pool.update(entry.id, keep).unwrap();
        assert_eq!(updated.password.as_deref(), Some("p1"));
        assert_eq!(updated.name, "renamed");
        assert_eq!(pool.health_of(entry.id).status, ProxyStatus::Healthy);

        // 传了新密码就覆盖；认证变化后健康状态重置为未知
        let mut overwrite = input("renamed", "http://a:1");
        overwrite.username = Some("u".into());
        overwrite.password = Some("p2".into());
        assert_eq!(
            pool.update(entry.id, overwrite).unwrap().password.as_deref(),
            Some("p2")
        );
        assert_eq!(pool.health_of(entry.id).status, ProxyStatus::Unknown);

        // 只清密码不清用户名 → 拒绝
        let mut clear_only = input("renamed", "http://a:1");
        clear_only.username = Some("u".into());
        clear_only.clear_password = true;
        assert!(matches!(
            pool.update(entry.id, clear_only),
            Err(ProxyPoolError::Invalid(_))
        ));
        assert_eq!(pool.get(entry.id).unwrap().password.as_deref(), Some("p2"));

        // 同时清用户名和密码
        let mut clear = input("renamed", "http://a:1");
        clear.clear_password = true;
        let cleared = pool.update(entry.id, clear).unwrap();
        assert_eq!(cleared.username, None);
        assert_eq!(cleared.password, None);
    }

    #[test]
    fn url_validation() {
        for ok in [
            "http://1.2.3.4:8080",
            "https://proxy.example.com:443",
            "socks5://127.0.0.1:1080",
            "socks5h://proxy.example.com:1080",
        ] {
            assert!(validate_proxy_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "direct",
            "ftp://1.2.3.4:21",
            "socks4://1.2.3.4:1080",
            "1.2.3.4:8080",
            "http://user:pass@1.2.3.4:8080",
        ] {
            assert!(validate_proxy_url(bad).is_err(), "{bad}");
        }
        assert!(ProxyPool::in_memory(None).add(input("", "http://a:1")).is_err());
    }

    fn parsed(line: &str) -> (String, String, Option<String>, Option<String>) {
        let input = parse_proxy_line(line, "socks5").unwrap();
        (input.name, input.url, input.username, input.password)
    }

    #[test]
    fn parse_line_url_with_auth() {
        assert_eq!(
            parsed("  http://alice:s3cret@10.0.0.1:3128  "),
            (
                "10.0.0.1:3128".into(),
                "http://10.0.0.1:3128".into(),
                Some("alice".into()),
                Some("s3cret".into())
            )
        );
        // 结果能通过 add 的校验（URL 不含认证信息）
        let pool = ProxyPool::in_memory(None);
        let entry = pool
            .add(parse_proxy_line("socks5h://u:p@proxy.example.com:1080", "http").unwrap())
            .unwrap();
        assert_eq!(entry.url, "socks5h://proxy.example.com:1080");
        assert_eq!(entry.name, "proxy.example.com:1080");
    }

    #[test]
    fn parse_line_url_without_auth() {
        assert_eq!(
            parsed("https://proxy.example.com:8443"),
            (
                "proxy.example.com:8443".into(),
                "https://proxy.example.com:8443".into(),
                None,
                None
            )
        );
        // 端口等于协议默认端口时仍保留
        assert_eq!(parsed("http://1.2.3.4:80").1, "http://1.2.3.4:80");
    }

    #[test]
    fn parse_line_host_port_with_default_scheme() {
        assert_eq!(
            parsed("1.2.3.4:1080:bob:pw"),
            (
                "1.2.3.4:1080".into(),
                "socks5://1.2.3.4:1080".into(),
                Some("bob".into()),
                Some("pw".into())
            )
        );
        let input = parse_proxy_line("gw.example.com:8080", "http").unwrap();
        assert_eq!(input.url, "http://gw.example.com:8080");
        assert_eq!(input.username, None);
        assert_eq!(input.password, None);
        // 密码里可以带冒号
        assert_eq!(parsed("1.2.3.4:1080:bob:a:b").3.as_deref(), Some("a:b"));
        // 非法默认协议
        assert!(parse_proxy_line("1.2.3.4:1080", "ftp").is_err());
    }

    #[test]
    fn parse_line_ipv6() {
        assert_eq!(
            parsed("[2001:db8::1]:1080:u:p"),
            (
                "[2001:db8::1]:1080".into(),
                "socks5://[2001:db8::1]:1080".into(),
                Some("u".into()),
                Some("p".into())
            )
        );
        assert_eq!(parsed("[::1]:8080").1, "socks5://[::1]:8080");
        assert_eq!(
            parsed("http://u:p@[2001:db8::2]:3128").1,
            "http://[2001:db8::2]:3128"
        );
        assert!(validate_proxy_url(&parsed("[::1]:8080").1).is_ok());
    }

    #[test]
    fn parse_line_percent_encoded_credentials() {
        let (_, url, username, password) = parsed("socks5://us%40er:p%40ss%3Aw%2Fd@1.2.3.4:1080");
        assert_eq!(url, "socks5://1.2.3.4:1080");
        assert_eq!(username.as_deref(), Some("us@er"));
        assert_eq!(password.as_deref(), Some("p@ss:w/d"));
    }

    #[test]
    fn parse_line_rejects_invalid() {
        for bad in [
            "not a proxy",
            "1.2.3.4",
            "1.2.3.4:",
            "1.2.3.4:abc",
            "1.2.3.4:70000",
            "1.2.3.4:0",
            "1.2.3.4:1080:onlyuser",
            "1.2.3.4:1080::pw",
            "2001:db8::1:1080",
            "[2001:db8::1]1080",
            "ftp://1.2.3.4:21",
            "http://1.2.3.4",
            "http://1.2.3.4:8080/path",
            "http://1.2.3.4:8080?x=1",
        ] {
            let error = parse_proxy_line(bad, "socks5").unwrap_err();
            assert!(matches!(error, ProxyPoolError::InvalidUrl(_)), "{bad}: {error:?}");
        }
        // 错误里带原文，截断到 80 字符，且不泄露密码
        let long = format!("{}:1", "x".repeat(200));
        let message = parse_proxy_line(&format!("{long}:u"), "socks5").unwrap_err().to_string();
        assert!(message.contains(&"x".repeat(80)));
        assert!(!message.contains(&"x".repeat(81)));
        let message = parse_proxy_line("http://u:topsecret@1.2.3.4", "socks5")
            .unwrap_err()
            .to_string();
        assert!(message.contains("http://u:***@1.2.3.4"), "{message}");
        assert!(!message.contains("topsecret"));
    }

    #[test]
    fn add_batch_dedups_against_pool_and_within_batch() {
        let pool = ProxyPool::in_memory(None);
        let mut existing = input("old", "http://10.0.0.1:3128");
        existing.username = Some("u".into());
        existing.password = Some("p".into());
        let existing = pool.add(existing).unwrap();

        let lines = [
            "HTTP://u:other@10.0.0.1:3128", // 与池中重复（scheme 大小写不同、密码不同）
            "10.0.0.2:1080",
            "socks5://10.0.0.2:1080", // 与第 2 行重复
            "http://10.0.0.1:3128",   // 同地址无用户名 → 不算重复
            "10.0.0.3:1080:u:p",
        ];
        let inputs = lines
            .iter()
            .enumerate()
            .map(|(i, line)| (i + 1, parse_proxy_line(line, "socks5").unwrap()))
            .chain([(9, input("", "http://10.0.0.9:1"))]) // 校验失败
            .collect();
        let result = pool.add_batch(inputs);

        assert_eq!(
            result.added.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["10.0.0.2:1080", "10.0.0.1:3128", "10.0.0.3:1080"]
        );
        assert_eq!(
            result.skipped,
            vec![
                BatchLineIssue {
                    line: 1,
                    reason: format!("已存在 #{}", existing.id)
                },
                BatchLineIssue {
                    line: 3,
                    reason: "与第 2 行重复".into()
                },
            ]
        );
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].line, 9);
        assert_eq!(pool.list().len(), 4);
        // ID 连续分配
        let ids: Vec<u64> = result.added.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![existing.id + 1, existing.id + 2, existing.id + 3]);

        // 手填 URL 的 host 大小写不同、显式写默认端口也视为重复
        let mixed = pool.add(input("mixed", "http://Proxy.Example.com")).unwrap();
        let again = pool.add_batch(vec![(1, input("x", "http://proxy.example.COM:80"))]);
        assert!(again.added.is_empty());
        assert_eq!(again.skipped[0].reason, format!("已存在 #{}", mixed.id));
    }

    #[test]
    fn add_batch_persists_once() {
        let dir = temp_dir();
        let config = Config::default();
        let pool = ProxyPool::load(&dir, None, &config).unwrap();
        let inputs = (1..=3)
            .map(|i| (i, parse_proxy_line(&format!("10.0.1.{i}:1080:u:p{i}"), "socks5").unwrap()))
            .collect();
        let result = pool.add_batch(inputs);
        assert_eq!(result.added.len(), 3);
        assert_eq!(
            pool.entries_persist_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        let reloaded = ProxyPool::load(&dir, None, &config).unwrap();
        assert_eq!(reloaded.list().len(), 3);
        assert_eq!(reloaded.get(result.added[2].id).unwrap().password.as_deref(), Some("p3"));

        // 全部跳过时不写文件
        let before = std::fs::metadata(dir.join("proxies.json")).unwrap().modified().unwrap();
        let again = pool.add_batch(vec![(1, parse_proxy_line("10.0.1.1:1080:u:x", "socks5").unwrap())]);
        assert!(again.added.is_empty());
        assert_eq!(again.skipped.len(), 1);
        let after = std::fs::metadata(dir.join("proxies.json")).unwrap().modified().unwrap();
        assert_eq!(before, after);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
