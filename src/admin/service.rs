//! Admin API 业务逻辑服务

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::accounts::{AccountStore, AuthUser, Role};
use crate::kiro::model::credentials::KiroCredentials;
use crate::kiro::token_manager::{MultiTokenManager, ProxyBinding};
use crate::proxy_pool::{
    BatchLineIssue, IMPORT_SCHEMES, PickError, ProxyInput, ProxyPool, ProxyPoolError,
    parse_proxy_line,
};

use super::error::AdminServiceError;
use super::types::{
    AddCredentialRequest, AddCredentialResponse, ArchiveCredentialRequest, BalanceResponse, CredentialEgressResponse,
    CredentialStatusItem, CredentialsStatusResponse, ImportProxiesRequest, ImportProxiesResponse,
    LoadBalancingModeResponse, ProxyItem, ProxyListResponse, SetCredentialProxyRequest,
    SetLoadBalancingModeRequest,
};

/// 代理池中没有健康代理时添加凭据的错误信息
const NO_HEALTHY_PROXY_MESSAGE: &str = "代理池中没有健康的代理，请先检查代理或显式指定 direct";

/// 余额缓存过期时间（秒），5 分钟
const BALANCE_CACHE_TTL_SECS: i64 = 300;

/// 缓存的余额条目（含时间戳）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedBalance {
    /// 缓存时间（Unix 秒）
    cached_at: f64,
    /// 缓存的余额数据
    data: BalanceResponse,
}

/// Admin 服务
///
/// 封装所有 Admin API 的业务逻辑
pub struct AdminService {
    token_manager: Arc<MultiTokenManager>,
    /// 出口代理池（与 token_manager、provider 共享同一实例）
    proxy_pool: Arc<ProxyPool>,
    account_store: Arc<AccountStore>,
    balance_cache: Mutex<HashMap<u64, CachedBalance>>,
    cache_path: Option<PathBuf>,
    /// 已注册的端点名称集合（用于 add_credential 校验）
    known_endpoints: HashSet<String>,
}

impl AdminService {
    pub fn new(
        token_manager: Arc<MultiTokenManager>,
        known_endpoints: impl IntoIterator<Item = String>,
        account_store: Arc<AccountStore>,
    ) -> Self {
        let cache_path = token_manager
            .cache_dir()
            .map(|d| d.join("kiro_balance_cache.json"));

        let balance_cache = Self::load_balance_cache_from(&cache_path);

        Self {
            proxy_pool: token_manager.proxy_pool(),
            token_manager,
            account_store,
            balance_cache: Mutex::new(balance_cache),
            cache_path,
            known_endpoints: known_endpoints.into_iter().collect(),
        }
    }

    /// 获取所有凭据状态
    ///
    /// `include_archived` 为 false 时过滤掉已归档凭据。
    /// `total` 只统计未归档凭据（已归档凭据一定是禁用的，不影响 `available`）。
    pub fn get_all_credentials(
        &self,
        actor: &AuthUser,
        include_archived: bool,
    ) -> Result<CredentialsStatusResponse, AdminServiceError> {
        let snapshot = self.token_manager.snapshot();
        let total = snapshot.entries.iter().filter(|e| !e.archived).count();
        let default_endpoint = self.token_manager.config().default_endpoint.clone();
        let manageable_owner_ids = self
            .account_store
            .manageable_account_ids(actor)
            .map_err(|error| AdminServiceError::InternalError(error.to_string()))?;

        let mut credentials: Vec<CredentialStatusItem> = snapshot
            .entries
            .into_iter()
            .filter(|entry| include_archived || !entry.archived)
            .map(|entry| CredentialStatusItem {
                id: entry.id,
                priority: entry.priority,
                disabled: entry.disabled,
                failure_count: entry.failure_count,
                is_current: entry.id == snapshot.current_id,
                expires_at: entry.expires_at,
                auth_method: entry.auth_method,
                has_profile_arn: entry.has_profile_arn,
                refresh_token_hash: entry.refresh_token_hash,
                api_key_hash: entry.api_key_hash,
                masked_api_key: entry.masked_api_key,
                email: entry.email,
                success_count: entry.success_count,
                last_used_at: entry.last_used_at.clone(),
                used_credits: entry.used_credits,
                has_proxy: entry.has_proxy,
                proxy_url: entry.proxy_url,
                proxy_id: entry.proxy_id,
                proxy_name: entry.proxy_name,
                egress_ip: entry.egress_ip,
                refresh_failure_count: entry.refresh_failure_count,
                registered_by: entry.registered_by,
                registered_by_name: entry.registered_by_name.clone(),
                can_manage: actor.role == Role::SuperAdmin
                    || entry
                        .registered_by
                        .is_some_and(|id| manageable_owner_ids.contains(&id)),
                disabled_reason: entry.disabled_reason,
                endpoint: entry.endpoint.unwrap_or_else(|| default_endpoint.clone()),
                archived: entry.archived,
                archived_at: entry.archived_at,
                archived_by: entry.archived_by,
                archive_reason: entry.archive_reason,
            })
            .collect();

        // 按优先级排序（数字越小优先级越高）
        credentials.sort_by_key(|c| c.priority);

        Ok(CredentialsStatusResponse {
            total,
            available: snapshot.available,
            current_id: snapshot.current_id,
            credentials,
        })
    }

    fn require_manage_permission(
        &self,
        actor: &AuthUser,
        id: u64,
    ) -> Result<(), AdminServiceError> {
        let owner_id = self
            .token_manager
            .snapshot()
            .entries
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or(AdminServiceError::NotFound { id })?
            .registered_by;
        if self
            .account_store
            .can_manage_credential(actor, owner_id)
            .map_err(|error| AdminServiceError::InternalError(error.to_string()))?
        {
            Ok(())
        } else {
            Err(AdminServiceError::Forbidden(
                "只能管理自己及下级代理登记的凭据".into(),
            ))
        }
    }

    /// 设置凭据禁用状态
    pub fn set_disabled(
        &self,
        actor: &AuthUser,
        id: u64,
        disabled: bool,
    ) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        // 先获取当前凭据 ID，用于判断是否需要切换
        let snapshot = self.token_manager.snapshot();
        let current_id = snapshot.current_id;

        self.token_manager
            .set_disabled(id, disabled)
            .map_err(|e| self.classify_error(e, id))?;

        // 只有禁用的是当前凭据时才尝试切换到下一个
        if disabled && id == current_id {
            let _ = self.token_manager.switch_to_next();
        }
        Ok(())
    }

    /// 设置凭据优先级
    pub fn set_priority(
        &self,
        actor: &AuthUser,
        id: u64,
        priority: u32,
    ) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        self.token_manager
            .set_priority(id, priority)
            .map_err(|e| self.classify_error(e, id))
    }

    /// 重置失败计数并重新启用
    pub fn reset_and_enable(&self, actor: &AuthUser, id: u64) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        self.token_manager
            .reset_and_enable(id)
            .map_err(|e| self.classify_error(e, id))
    }

    /// 获取凭据余额（带缓存）
    pub async fn get_balance(&self, id: u64) -> Result<BalanceResponse, AdminServiceError> {
        // 先查缓存
        {
            let cache = self.balance_cache.lock();
            if let Some(cached) = cache.get(&id) {
                let now = Utc::now().timestamp() as f64;
                if (now - cached.cached_at) < BALANCE_CACHE_TTL_SECS as f64 {
                    tracing::debug!("凭据 #{} 余额命中缓存", id);
                    return Ok(cached.data.clone());
                }
            }
        }

        // 缓存未命中或已过期，从上游获取
        let balance = self.fetch_balance(id).await?;

        // 更新缓存
        {
            let mut cache = self.balance_cache.lock();
            cache.insert(
                id,
                CachedBalance {
                    cached_at: Utc::now().timestamp() as f64,
                    data: balance.clone(),
                },
            );
        }
        self.save_balance_cache();

        Ok(balance)
    }

    /// 从上游获取余额（无缓存）
    async fn fetch_balance(&self, id: u64) -> Result<BalanceResponse, AdminServiceError> {
        let usage = self
            .token_manager
            .get_usage_limits_for(id)
            .await
            .map_err(|e| self.classify_balance_error(e, id))?;

        let current_usage = usage.current_usage();
        let usage_limit = usage.usage_limit();
        let remaining = (usage_limit - current_usage).max(0.0);
        let usage_percentage = if usage_limit > 0.0 {
            (current_usage / usage_limit * 100.0).min(100.0)
        } else {
            0.0
        };

        Ok(BalanceResponse {
            id,
            subscription_title: usage.subscription_title().map(|s| s.to_string()),
            current_usage,
            usage_limit,
            remaining,
            usage_percentage,
            next_reset_at: usage.next_date_reset,
        })
    }

    /// 添加新凭据
    pub async fn add_credential(
        &self,
        actor: &AuthUser,
        req: AddCredentialRequest,
    ) -> Result<AddCredentialResponse, AdminServiceError> {
        // 校验端点名：未指定则默认合法，指定则必须已注册
        if let Some(ref name) = req.endpoint {
            if !self.known_endpoints.contains(name) {
                let mut known: Vec<&str> =
                    self.known_endpoints.iter().map(|s| s.as_str()).collect();
                known.sort();
                return Err(AdminServiceError::InvalidCredential(format!(
                    "未知端点 \"{}\"，已注册端点: {:?}",
                    name, known
                )));
            }
        }

        // 代理：proxyId 与 proxyUrl 互斥；都不传时从健康代理中自动分配
        let proxy_url = req
            .proxy_url
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty());
        let proxy_id = match (req.proxy_id, proxy_url.as_deref()) {
            (Some(_), Some(_)) => {
                return Err(AdminServiceError::InvalidRequest(
                    "proxyId 与 proxyUrl 不能同时指定".into(),
                ));
            }
            (Some(proxy_id), None) => {
                self.require_enabled_proxy(proxy_id)?;
                Some(proxy_id)
            }
            // 显式直填代理或 direct：不走自动分配
            (None, Some(_)) => None,
            (None, None) => {
                // 每次调用都基于当前快照重新计数，批量导入逐条调用时自然均衡
                let bound_counts = self.bound_counts();
                match self.proxy_pool.pick_for_new_credential(&bound_counts) {
                    Ok(picked) => picked,
                    Err(PickError::NoHealthyProxy) => {
                        return Err(AdminServiceError::InvalidRequest(
                            NO_HEALTHY_PROXY_MESSAGE.into(),
                        ));
                    }
                }
            }
        };
        let (proxy_username, proxy_password) = if proxy_url.is_some() {
            (req.proxy_username, req.proxy_password)
        } else {
            (None, None)
        };

        // 构建凭据对象
        let email = req.email.clone();
        let new_cred = KiroCredentials {
            id: None,
            access_token: None,
            refresh_token: req.refresh_token,
            profile_arn: req.profile_arn,
            expires_at: None,
            auth_method: Some(req.auth_method),
            client_id: req.client_id,
            client_secret: req.client_secret,
            priority: req.priority,
            region: req.region,
            auth_region: req.auth_region,
            api_region: req.api_region,
            machine_id: req.machine_id,
            email: req.email,
            subscription_title: None, // 将在首次获取使用额度时自动更新
            proxy_url,
            proxy_username,
            proxy_password,
            proxy_id,
            disabled: false, // 新添加的凭据默认启用
            kiro_api_key: req.kiro_api_key,
            registered_by: Some(actor.id),
            registered_by_name: Some(actor.username.clone()),
            endpoint: req.endpoint,
            archived: false,
            archived_at: None,
            archived_by: None,
            archive_reason: None,
        };

        // 调用 token_manager 添加凭据
        let credential_id = self
            .token_manager
            .add_credential(new_cred)
            .await
            .map_err(|e| self.classify_add_error(e))?;

        // 主动获取订阅等级，避免首次请求时 Free 账号绕过 Opus 模型过滤
        #[cfg(not(test))]
        if let Err(e) = self.token_manager.get_usage_limits_for(credential_id).await {
            tracing::warn!("添加凭据后获取订阅等级失败（不影响凭据添加）: {}", e);
        }

        Ok(AddCredentialResponse {
            success: true,
            message: format!("凭据添加成功，ID: {}", credential_id),
            credential_id,
            email,
        })
    }

    // ============ 出口代理池 ============

    /// 未归档凭据按 proxy_id 计数
    fn bound_counts(&self) -> HashMap<u64, usize> {
        let mut counts = HashMap::new();
        for entry in self.token_manager.snapshot().entries {
            if entry.archived {
                continue;
            }
            if let Some(proxy_id) = entry.proxy_id {
                *counts.entry(proxy_id).or_insert(0) += 1;
            }
        }
        counts
    }

    fn require_enabled_proxy(&self, proxy_id: u64) -> Result<(), AdminServiceError> {
        match self.proxy_pool.get(proxy_id) {
            Some(entry) if entry.enabled => Ok(()),
            Some(_) => Err(AdminServiceError::InvalidRequest(format!(
                "代理 #{} 已停用",
                proxy_id
            ))),
            None => Err(AdminServiceError::InvalidRequest(format!(
                "代理 #{} 不存在",
                proxy_id
            ))),
        }
    }

    fn proxy_item(
        &self,
        entry: crate::proxy_pool::ProxyEntry,
        health: crate::proxy_pool::ProxyHealth,
        bound_counts: &HashMap<u64, usize>,
    ) -> ProxyItem {
        ProxyItem {
            id: entry.id,
            bound_count: bound_counts.get(&entry.id).copied().unwrap_or(0),
            name: entry.name,
            url: entry.url,
            username: entry.username,
            password_set: entry.password.is_some(),
            note: entry.note,
            enabled: entry.enabled,
            created_at: entry.created_at,
            health,
        }
    }

    fn single_proxy_item(&self, id: u64) -> Result<ProxyItem, AdminServiceError> {
        let entry = self
            .proxy_pool
            .get(id)
            .ok_or(AdminServiceError::ProxyNotFound { id })?;
        let health = self.proxy_pool.health_of(id);
        Ok(self.proxy_item(entry, health, &self.bound_counts()))
    }

    /// 代理列表（含健康状态与绑定数，不含密码）
    pub fn list_proxies(&self) -> ProxyListResponse {
        let bound_counts = self.bound_counts();
        ProxyListResponse {
            proxies: self
                .proxy_pool
                .list()
                .into_iter()
                .map(|(entry, health)| self.proxy_item(entry, health, &bound_counts))
                .collect(),
        }
    }

    pub fn add_proxy(&self, input: ProxyInput) -> Result<ProxyItem, AdminServiceError> {
        let entry = self.proxy_pool.add(input).map_err(map_pool_error)?;
        self.single_proxy_item(entry.id)
    }

    pub fn update_proxy(&self, id: u64, input: ProxyInput) -> Result<ProxyItem, AdminServiceError> {
        self.proxy_pool.update(id, input).map_err(map_pool_error)?;
        self.single_proxy_item(id)
    }

    /// 删除代理：仍有未归档凭据绑定时拒绝
    pub fn remove_proxy(&self, id: u64) -> Result<(), AdminServiceError> {
        let bound = self.bound_counts().get(&id).copied().unwrap_or(0);
        self.proxy_pool.remove(id, bound).map_err(map_pool_error)
    }

    pub fn set_proxy_enabled(&self, id: u64, enabled: bool) -> Result<ProxyItem, AdminServiceError> {
        self.proxy_pool
            .set_enabled(id, enabled)
            .map_err(map_pool_error)?;
        self.single_proxy_item(id)
    }

    pub async fn check_proxy(&self, id: u64) -> Result<ProxyItem, AdminServiceError> {
        self.proxy_pool.check(id).await.map_err(map_pool_error)?;
        self.single_proxy_item(id)
    }

    /// 批量导入代理：逐行解析（空行和 `#` 注释跳过），一次性加入后可选并发检测
    pub async fn import_proxies(
        &self,
        req: ImportProxiesRequest,
    ) -> Result<ImportProxiesResponse, AdminServiceError> {
        let default_scheme = req
            .default_scheme
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "socks5".to_string());
        if !IMPORT_SCHEMES.contains(&default_scheme.as_str()) {
            return Err(AdminServiceError::InvalidRequest(
                "defaultScheme 必须是 http、https、socks5 或 socks5h".into(),
            ));
        }

        let lines: Vec<&str> = req.text.lines().collect();
        let mut inputs = Vec::new();
        let mut failed = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match parse_proxy_line(line, &default_scheme) {
                Ok(input) => inputs.push((index + 1, input)),
                Err(e) => failed.push(BatchLineIssue {
                    line: index + 1,
                    reason: e.to_string(),
                }),
            }
        }

        let result = self.proxy_pool.add_batch(inputs);
        failed.extend(result.failed);
        failed.sort_by_key(|issue| issue.line);

        let ids: Vec<u64> = result.added.iter().map(|e| e.id).collect();
        if req.check_after_import.unwrap_or(true) && !ids.is_empty() {
            self.proxy_pool.check_many(ids).await;
        }
        let bound_counts = self.bound_counts();
        let added = result
            .added
            .into_iter()
            .map(|entry| {
                let health = self.proxy_pool.health_of(entry.id);
                self.proxy_item(entry, health, &bound_counts)
            })
            .collect();

        Ok(ImportProxiesResponse {
            added,
            skipped: result.skipped,
            failed,
            total_lines: lines.len(),
        })
    }

    pub async fn check_all_proxies(&self) -> ProxyListResponse {
        self.proxy_pool.check_all().await;
        self.list_proxies()
    }

    /// 修改凭据代理绑定（三选一：proxyId / direct / clear）
    pub fn set_credential_proxy(
        &self,
        actor: &AuthUser,
        id: u64,
        req: SetCredentialProxyRequest,
    ) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        let selected = [req.proxy_id.is_some(), req.direct, req.clear]
            .iter()
            .filter(|v| **v)
            .count();
        if selected != 1 {
            return Err(AdminServiceError::InvalidRequest(
                "proxyId、direct、clear 必须且只能指定一个".into(),
            ));
        }
        let binding = if let Some(proxy_id) = req.proxy_id {
            self.require_enabled_proxy(proxy_id)?;
            ProxyBinding::Pool(proxy_id)
        } else if req.direct {
            ProxyBinding::Direct
        } else {
            ProxyBinding::Clear
        };
        self.token_manager
            .set_proxy_binding(id, binding)
            .map_err(|e| self.classify_error(e, id))
    }

    /// 用凭据实际使用的代理做一次出口检测（不写入代理池健康状态）
    pub async fn check_credential_egress(
        &self,
        id: u64,
    ) -> Result<CredentialEgressResponse, AdminServiceError> {
        let credentials = self
            .token_manager
            .credentials_of(id)
            .ok_or(AdminServiceError::NotFound { id })?;
        let proxy = self.proxy_pool.resolve(&credentials);
        let health = self.proxy_pool.check_with(proxy.as_ref()).await;
        Ok(CredentialEgressResponse {
            credential_id: id,
            proxy_url: proxy.map(|p| display_proxy_url(&p.url)),
            health,
        })
    }

    /// 归档凭据（保留统计与余额缓存，可恢复）
    pub fn archive_credential(
        &self,
        actor: &AuthUser,
        id: u64,
        req: ArchiveCredentialRequest,
    ) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        let reason = req
            .reason
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty());
        self.token_manager
            .archive_credential(id, actor.id, reason)
            .map_err(|e| self.classify_error(e, id))
    }

    /// 恢复已归档凭据（恢复后仍为禁用状态）
    pub fn unarchive_credential(&self, actor: &AuthUser, id: u64) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        self.token_manager
            .unarchive_credential(id)
            .map_err(|e| self.classify_error(e, id))
    }

    /// 彻底删除已归档凭据（仅超级管理员，Token、统计、累计消耗和余额缓存一并删除，无法恢复）
    pub fn purge_credential(&self, actor: &AuthUser, id: u64) -> Result<(), AdminServiceError> {
        if actor.role != Role::SuperAdmin {
            return Err(AdminServiceError::Forbidden(
                "仅超级管理员可以彻底删除凭据".into(),
            ));
        }
        self.token_manager
            .purge_credential(id)
            .map_err(|e| self.classify_error(e, id))?;

        // 清理已删除凭据的余额缓存
        {
            let mut cache = self.balance_cache.lock();
            cache.remove(&id);
        }
        self.save_balance_cache();

        Ok(())
    }

    /// 获取负载均衡模式
    pub fn get_load_balancing_mode(&self) -> LoadBalancingModeResponse {
        LoadBalancingModeResponse {
            mode: self.token_manager.get_load_balancing_mode(),
        }
    }

    /// 设置负载均衡模式
    pub fn set_load_balancing_mode(
        &self,
        actor: &AuthUser,
        req: SetLoadBalancingModeRequest,
    ) -> Result<LoadBalancingModeResponse, AdminServiceError> {
        if actor.role != Role::SuperAdmin {
            return Err(AdminServiceError::Forbidden(
                "仅超级管理员可以修改负载均衡模式".into(),
            ));
        }
        // 验证模式值
        if req.mode != "priority" && req.mode != "balanced" {
            return Err(AdminServiceError::InvalidCredential(
                "mode 必须是 'priority' 或 'balanced'".to_string(),
            ));
        }

        self.token_manager
            .set_load_balancing_mode(req.mode.clone())
            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        Ok(LoadBalancingModeResponse { mode: req.mode })
    }

    /// 强制刷新指定凭据的 Token
    pub async fn force_refresh_token(
        &self,
        actor: &AuthUser,
        id: u64,
    ) -> Result<(), AdminServiceError> {
        self.require_manage_permission(actor, id)?;
        self.token_manager
            .force_refresh_token_for(id)
            .await
            .map_err(|e| self.classify_balance_error(e, id))
    }

    // ============ 余额缓存持久化 ============

    fn load_balance_cache_from(cache_path: &Option<PathBuf>) -> HashMap<u64, CachedBalance> {
        let path = match cache_path {
            Some(p) => p,
            None => return HashMap::new(),
        };

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return HashMap::new(),
        };

        // 文件中使用字符串 key 以兼容 JSON 格式
        let map: HashMap<String, CachedBalance> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("解析余额缓存失败，将忽略: {}", e);
                return HashMap::new();
            }
        };

        let now = Utc::now().timestamp() as f64;
        map.into_iter()
            .filter_map(|(k, v)| {
                let id = k.parse::<u64>().ok()?;
                // 丢弃超过 TTL 的条目
                if (now - v.cached_at) < BALANCE_CACHE_TTL_SECS as f64 {
                    Some((id, v))
                } else {
                    None
                }
            })
            .collect()
    }

    fn save_balance_cache(&self) {
        let path = match &self.cache_path {
            Some(p) => p,
            None => return,
        };

        // 持有锁期间完成序列化和写入，防止并发损坏
        let cache = self.balance_cache.lock();
        let map: HashMap<String, &CachedBalance> =
            cache.iter().map(|(k, v)| (k.to_string(), v)).collect();

        match serde_json::to_string_pretty(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    tracing::warn!("保存余额缓存失败: {}", e);
                }
            }
            Err(e) => tracing::warn!("序列化余额缓存失败: {}", e),
        }
    }

    // ============ 错误分类 ============

    /// 分类简单操作错误（set_disabled, set_priority, reset_and_enable）
    fn classify_error(&self, e: anyhow::Error, id: u64) -> AdminServiceError {
        let msg = e.to_string();
        if msg.contains("不存在") {
            AdminServiceError::NotFound { id }
        } else if msg.contains("已归档") || msg.contains("未归档") {
            AdminServiceError::InvalidCredential(msg)
        } else {
            AdminServiceError::InternalError(msg)
        }
    }

    /// 分类余额查询错误（可能涉及上游 API 调用）
    fn classify_balance_error(&self, e: anyhow::Error, id: u64) -> AdminServiceError {
        let msg = e.to_string();

        // 1. 凭据不存在
        if msg.contains("不存在") {
            return AdminServiceError::NotFound { id };
        }

        // 2. API Key 凭据不支持刷新：客户端请求错误，映射为 400
        if msg.contains("API Key 凭据不支持刷新") {
            return AdminServiceError::InvalidCredential(msg);
        }

        // 3. 上游服务错误特征：HTTP 响应错误或网络错误
        let is_upstream_error =
            // HTTP 响应错误（来自 refresh_*_token 的错误消息）
            msg.contains("凭证已过期或无效") ||
            msg.contains("权限不足") ||
            msg.contains("已被限流") ||
            msg.contains("服务器错误") ||
            msg.contains("Token 刷新失败") ||
            msg.contains("暂时不可用") ||
            // 网络错误（reqwest 错误）
            msg.contains("error trying to connect") ||
            msg.contains("connection") ||
            msg.contains("timeout") ||
            msg.contains("timed out");

        if is_upstream_error {
            AdminServiceError::UpstreamError(msg)
        } else {
            // 4. 默认归类为内部错误（本地验证失败、配置错误等）
            // 包括：缺少 refreshToken、refreshToken 已被截断、无法生成 machineId 等
            AdminServiceError::InternalError(msg)
        }
    }

    /// 分类添加凭据错误
    fn classify_add_error(&self, e: anyhow::Error) -> AdminServiceError {
        let msg = e.to_string();

        // 凭据验证失败（refreshToken 无效、格式错误等）
        let is_invalid_credential = msg.contains("缺少 refreshToken")
            || msg.contains("refreshToken 为空")
            || msg.contains("refreshToken 已被截断")
            || msg.contains("凭据已存在")
            || msg.contains("该凭据已归档")
            || msg.contains("refreshToken 重复")
            || msg.contains("kiroApiKey 重复")
            || msg.contains("缺少 kiroApiKey")
            || msg.contains("kiroApiKey 为空")
            || msg.contains("凭证已过期或无效")
            || msg.contains("权限不足")
            || msg.contains("已被限流");

        if is_invalid_credential {
            AdminServiceError::InvalidCredential(msg)
        } else if msg.contains("error trying to connect")
            || msg.contains("connection")
            || msg.contains("timeout")
        {
            AdminServiceError::UpstreamError(msg)
        } else {
            AdminServiceError::InternalError(msg)
        }
    }
}

fn map_pool_error(error: ProxyPoolError) -> AdminServiceError {
    match error {
        ProxyPoolError::NotFound(id) => AdminServiceError::ProxyNotFound { id },
        ProxyPoolError::Invalid(_)
        | ProxyPoolError::InvalidUrl(_)
        | ProxyPoolError::InUse { .. } => {
            AdminServiceError::InvalidRequest(error.to_string())
        }
        ProxyPoolError::Storage(_) => AdminServiceError::InternalError(error.to_string()),
    }
}

/// 去掉代理 URL 中可能内嵌的认证信息，仅用于展示
fn display_proxy_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string().trim_end_matches('/').to_string()
        }
        Err(_) => "已配置".to_string(),
    }
}
