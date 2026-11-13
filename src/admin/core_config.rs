use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::accounts::AuthUser;
use crate::anthropic::AppState;
use crate::http_client::build_client;
use crate::kiro::provider::KiroProvider;
use crate::kiro::token_manager::MultiTokenManager;
use crate::model::config::{Config, TlsBackend};
use crate::runtime_config::RuntimeConfig;

use super::error::AdminServiceError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreConfigResponse {
    pub region: String,
    pub auth_region: Option<String>,
    pub api_region: Option<String>,
    pub kiro_version: String,
    pub system_version: String,
    pub node_version: String,
    pub default_endpoint: String,
    pub load_balancing_mode: String,
    pub tls_backend: TlsBackend,
    pub extract_thinking: bool,
    pub proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_username: Option<String>,
    pub proxy_password_set: bool,
    pub revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCoreConfigRequest {
    pub revision: u64,
    pub region: String,
    pub auth_region: Option<String>,
    pub api_region: Option<String>,
    pub kiro_version: String,
    pub system_version: String,
    pub node_version: String,
    pub default_endpoint: String,
    pub load_balancing_mode: String,
    pub tls_backend: TlsBackend,
    pub extract_thinking: bool,
    pub proxy_url: Option<String>,
    pub proxy_username: Option<String>,
    pub clear_proxy_username: Option<bool>,
    pub proxy_password: Option<String>,
    pub clear_proxy_password: Option<bool>,
}

pub struct CoreConfigService {
    runtime: Arc<RuntimeConfig>,
    token_manager: Arc<MultiTokenManager>,
    anthropic: Arc<RwLock<AppState>>,
    known_endpoints: Vec<String>,
    save_lock: parking_lot::Mutex<()>,
}

impl CoreConfigService {
    pub fn new(
        runtime: Arc<RuntimeConfig>,
        token_manager: Arc<MultiTokenManager>,
        anthropic: Arc<RwLock<AppState>>,
        known_endpoints: Vec<String>,
    ) -> Self {
        Self {
            runtime,
            token_manager,
            anthropic,
            known_endpoints,
            save_lock: parking_lot::Mutex::new(()),
        }
    }

    pub fn get(&self) -> CoreConfigResponse {
        let config = self.runtime.get();
        CoreConfigResponse {
            region: config.region.clone(),
            auth_region: config.auth_region.clone(),
            api_region: config.api_region.clone(),
            kiro_version: config.kiro_version.clone(),
            system_version: config.system_version.clone(),
            node_version: config.node_version.clone(),
            default_endpoint: config.default_endpoint.clone(),
            load_balancing_mode: config.load_balancing_mode.clone(),
            tls_backend: config.tls_backend,
            extract_thinking: config.extract_thinking,
            proxy_url: config.proxy_url.as_deref().map(|url| {
                url.split("://")
                    .nth(1)
                    .and_then(|address| address.rsplit_once(':'))
                    .map(|(host, port)| {
                        format!(
                            "{}://{}:{}",
                            url.split("://").next().unwrap_or("http"),
                            host,
                            port
                        )
                    })
                    .unwrap_or_else(|| "已配置".into())
            }),
            proxy_username: None,
            proxy_password_set: config.proxy_password.is_some(),
            revision: self.runtime.revision(),
        }
    }

    pub fn update(
        &self,
        actor: &AuthUser,
        request: UpdateCoreConfigRequest,
    ) -> Result<CoreConfigResponse, AdminServiceError> {
        let _save = self.save_lock.lock();
        if request.revision != self.runtime.revision() {
            return Err(AdminServiceError::InvalidCredential(
                "配置已被其他管理员修改，请刷新后重新编辑".into(),
            ));
        }
        if request.region.trim().is_empty()
            || !request
                .region
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(AdminServiceError::InvalidCredential(
                "Region 只能包含字母、数字和连字符".into(),
            ));
        }
        if request.kiro_version.trim().is_empty() || request.kiro_version.len() > 64 {
            return Err(AdminServiceError::InvalidCredential(
                "Kiro 客户端版本不能为空，且不能超过 64 个字符".into(),
            ));
        }
        if request.system_version.trim().is_empty()
            || request.system_version.len() > 128
            || request.node_version.trim().is_empty()
            || request.node_version.len() > 64
        {
            return Err(AdminServiceError::InvalidCredential(
                "客户端版本标识不能为空，且长度超出限制".into(),
            ));
        }
        if !self
            .known_endpoints
            .iter()
            .any(|name| name == &request.default_endpoint)
        {
            return Err(AdminServiceError::InvalidCredential(format!(
                "未知默认端点: {}",
                request.default_endpoint
            )));
        }
        if request.load_balancing_mode != "priority" && request.load_balancing_mode != "balanced" {
            return Err(AdminServiceError::InvalidCredential(
                "负载均衡模式必须是 priority 或 balanced".into(),
            ));
        }
        #[cfg(not(feature = "native-tls"))]
        if request.tls_backend == TlsBackend::NativeTls {
            return Err(AdminServiceError::InvalidCredential(
                "当前构建不包含 native-tls，请使用 rustls".into(),
            ));
        }
        let proxy_url = request
            .proxy_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string);
        if let Some(url) = &proxy_url {
            if !["http", "https", "socks5"]
                .iter()
                .any(|scheme| url.starts_with(&format!("{scheme}://")))
            {
                return Err(AdminServiceError::InvalidCredential(
                    "代理 URL 必须以 http://、https:// 或 socks5:// 开头".into(),
                ));
            }
            if url.contains('@') {
                return Err(AdminServiceError::InvalidCredential(
                    "代理认证信息请使用独立用户名和密码字段，不要写入 URL".into(),
                ));
            }
        }
        if request.clear_proxy_password == Some(true)
            && request
                .proxy_password
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        {
            return Err(AdminServiceError::InvalidCredential(
                "不能同时设置新代理密码和清除认证".into(),
            ));
        }
        let path = self.runtime.path().to_path_buf();
        let _file_lock =
            crate::shared_files::acquire(&path.with_extension("lock")).map_err(internal)?;
        let mut next = Config::load(&path).map_err(internal)?;
        next.region = request.region;
        next.auth_region = request.auth_region;
        next.api_region = request.api_region;
        next.kiro_version = request.kiro_version;
        next.system_version = request.system_version;
        next.node_version = request.node_version;
        next.default_endpoint = request.default_endpoint;
        next.load_balancing_mode = request.load_balancing_mode;
        next.tls_backend = request.tls_backend;
        next.extract_thinking = request.extract_thinking;
        next.proxy_url = proxy_url;
        next.proxy_username = request
            .proxy_username
            .or_else(|| next.proxy_username.clone());
        if request.clear_proxy_username == Some(true) {
            next.proxy_username = None;
        }
        if request.clear_proxy_password == Some(true) {
            next.proxy_password = None;
        } else if let Some(password) = request.proxy_password.filter(|value| !value.is_empty()) {
            next.proxy_password = Some(password);
        }
        if next.proxy_url.is_none() {
            next.proxy_username = None;
            next.proxy_password = None;
        }
        if next.proxy_username.is_some() != next.proxy_password.is_some() {
            return Err(AdminServiceError::InvalidCredential(
                "代理用户名和密码需要同时填写，或留空保留已有认证".into(),
            ));
        }
        if let Err(error) = build_client(
            crate::runtime_config::proxy_from_config(&next).as_ref(),
            10,
            next.tls_backend,
        ) {
            return Err(AdminServiceError::InvalidCredential(format!(
                "代理或 TLS 配置无效: {error}"
            )));
        }
        let backup = path.with_extension(format!("backup-{}.json", Utc::now().timestamp_millis()));
        std::fs::copy(&path, &backup).map_err(internal)?;
        if let Err(error) = next.save_locked() {
            let _ = std::fs::remove_file(backup);
            return Err(internal(error));
        }
        self.token_manager.replace(next.clone());
        if let Some(provider_lock) = self.anthropic.read().kiro_provider.as_ref() {
            let endpoints = provider_lock.read().endpoints().clone();
            let new_provider = KiroProvider::with_proxy(
                self.token_manager.clone(),
                crate::runtime_config::proxy_from_config(&next),
                endpoints,
                next.default_endpoint.clone(),
            );
            *provider_lock.write() = Arc::new(new_provider);
        }
        self.anthropic
            .read()
            .extract_thinking
            .store(next.extract_thinking, std::sync::atomic::Ordering::Relaxed);
        let _ = std::fs::remove_file(backup);
        tracing::info!(actor = %actor.username, revision = self.runtime.revision(), "核心配置已保存并热重载");
        Ok(self.get())
    }
}

fn internal(error: impl std::fmt::Display) -> AdminServiceError {
    AdminServiceError::InternalError(error.to_string())
}

#[cfg(all(test, not(feature = "mysql")))]
mod tests {
    use super::*;
    use crate::accounts::Role;

    fn service() -> (
        CoreConfigService,
        Arc<RuntimeConfig>,
        Arc<MultiTokenManager>,
        Arc<RwLock<AppState>>,
        std::path::PathBuf,
    ) {
        let dir = std::env::temp_dir().join(format!("kiro-core-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let mut config = Config::default();
        config.api_key = Some("secret-api-key".into());
        config.admin_api_key = Some("secret-admin-key".into());
        config.proxy_password = Some("secret-proxy-password".into());
        config.proxy_username = Some("proxy-user".into());
        config.proxy_url = Some("http://127.0.0.1:7890".into());
        std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let loaded = Config::load(&path).unwrap();
        let runtime = RuntimeConfig::from_config(loaded.clone());
        let manager =
            Arc::new(MultiTokenManager::new(runtime.clone(), vec![], None, None, false).unwrap());
        let state = Arc::new(RwLock::new(AppState::new("", true)));
        let service = CoreConfigService::new(
            runtime.clone(),
            manager.clone(),
            state.clone(),
            vec!["ide".into()],
        );
        (service, runtime, manager, state, dir)
    }

    fn request(service: &CoreConfigService) -> UpdateCoreConfigRequest {
        let current = service.get();
        UpdateCoreConfigRequest {
            revision: current.revision,
            region: current.region,
            auth_region: current.auth_region,
            api_region: current.api_region,
            kiro_version: current.kiro_version,
            system_version: current.system_version,
            node_version: current.node_version,
            default_endpoint: current.default_endpoint,
            load_balancing_mode: current.load_balancing_mode,
            tls_backend: current.tls_backend,
            extract_thinking: current.extract_thinking,
            proxy_url: current.proxy_url,
            proxy_username: current.proxy_username,
            clear_proxy_username: None,
            proxy_password: None,
            clear_proxy_password: None,
        }
    }

    fn admin() -> AuthUser {
        AuthUser {
            id: 1,
            username: "superadmin".into(),
            role: Role::SuperAdmin,
            parent_id: None,
            balance_micro: 0,
            enabled: true,
        }
    }

    #[test]
    fn updates_runtime_and_preserves_secrets_and_old_snapshot() {
        let (service, runtime, manager, state, _dir) = service();
        let before = runtime.get();
        let mut request = request(&service);
        request.kiro_version = "9.8.7".into();
        request.extract_thinking = false;
        request.load_balancing_mode = "balanced".into();
        let result = service.update(&admin(), request).unwrap();
        assert_eq!(result.kiro_version, "9.8.7");
        assert_eq!(result.revision, 2);
        assert_eq!(before.kiro_version, Config::default().kiro_version);
        assert!(
            !state
                .read()
                .extract_thinking
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        assert_eq!(manager.get_load_balancing_mode(), "balanced");
        assert!(result.proxy_password_set);
        let persisted = Config::load(runtime.path()).unwrap();
        assert_eq!(persisted.api_key.as_deref(), Some("secret-api-key"));
        assert_eq!(persisted.admin_api_key.as_deref(), Some("secret-admin-key"));
        assert_eq!(
            persisted.proxy_password.as_deref(),
            Some("secret-proxy-password")
        );
        assert!(!serde_json::to_string(&result).unwrap().contains("secret-"));
    }

    #[test]
    fn rejects_stale_revision_without_changing_config() {
        let (service, runtime, _manager, _state, _dir) = service();
        let mut request = request(&service);
        request.revision += 1;
        request.kiro_version = "untrusted-change".into();
        assert!(service.update(&admin(), request).is_err());
        assert_eq!(runtime.get().kiro_version, Config::default().kiro_version);
    }

    #[test]
    fn rejects_embedded_proxy_credentials() {
        let (service, _runtime, _manager, _state, _dir) = service();
        let mut request = request(&service);
        request.proxy_url = Some("http://user:password@localhost:7890".into());
        assert!(service.update(&admin(), request).is_err());
    }
}
