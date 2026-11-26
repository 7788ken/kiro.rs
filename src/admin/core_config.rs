use std::sync::Arc;

use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::accounts::AuthUser;
use crate::anthropic::AppState;
use crate::http_client::build_client;
use crate::kiro::provider::KiroProvider;
use crate::kiro::token_manager::MultiTokenManager;
use crate::model::config::{Config, InputTokenSource, PromptCacheMode, TlsBackend};
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
    pub site_name: Option<String>,
    pub logo_data_url: Option<String>,
    pub enforce_max_tokens: bool,
    pub input_token_source: InputTokenSource,
    pub prompt_cache_mode: PromptCacheMode,
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
    #[serde(default)]
    pub site_name: Option<String>,
    #[serde(default)]
    pub logo_data_url: Option<String>,
    #[serde(default)]
    pub clear_logo: Option<bool>,
    /// 未传时保留原值，便于旧客户端不覆盖这些开关
    #[serde(default)]
    pub enforce_max_tokens: Option<bool>,
    #[serde(default)]
    pub input_token_source: Option<InputTokenSource>,
    #[serde(default)]
    pub prompt_cache_mode: Option<PromptCacheMode>,
}

/// 站点名称最大字符数
pub const MAX_SITE_NAME_CHARS: usize = 60;
/// Logo data URL 整串最大字节数（200 KB）
pub const MAX_LOGO_DATA_URL_BYTES: usize = 200 * 1024;
const LOGO_MIME_TYPES: &[&str] = &[
    "image/svg+xml",
    "image/png",
    "image/jpeg",
    "image/x-icon",
    "image/webp",
];

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
            site_name: config.site_name.clone(),
            logo_data_url: config.logo_data_url.clone(),
            enforce_max_tokens: config.enforce_max_tokens,
            input_token_source: config.input_token_source,
            prompt_cache_mode: config.prompt_cache_mode,
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
        // 未传 siteName 保留原值；传空串（trim 后）表示清除
        let site_name = request
            .site_name
            .as_deref()
            .map(|name| Some(name.trim().to_string()).filter(|name| !name.is_empty()));
        if site_name
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|name| name.chars().count() > MAX_SITE_NAME_CHARS)
        {
            return Err(AdminServiceError::InvalidCredential(format!(
                "站点名称不能超过 {MAX_SITE_NAME_CHARS} 个字符"
            )));
        }
        let clear_logo = request.clear_logo == Some(true);
        if !clear_logo && let Some(logo) = &request.logo_data_url {
            validate_logo_data_url(logo)?;
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
        if let Some(site_name) = site_name {
            next.site_name = site_name;
        }
        if clear_logo {
            next.logo_data_url = None;
        } else if let Some(logo) = request.logo_data_url {
            next.logo_data_url = Some(logo);
        }
        if let Some(enforce) = request.enforce_max_tokens {
            next.enforce_max_tokens = enforce;
        }
        if let Some(source) = request.input_token_source {
            next.input_token_source = source;
        }
        if let Some(mode) = request.prompt_cache_mode {
            next.prompt_cache_mode = mode;
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
            let (endpoints, version_check) = {
                let current = provider_lock.read();
                (current.endpoints().clone(), current.version_check())
            };
            let new_provider = KiroProvider::with_proxy(
                self.token_manager.clone(),
                crate::runtime_config::proxy_from_config(&next),
                endpoints,
                next.default_endpoint.clone(),
            )
            .with_version_check(version_check);
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

/// 校验 Logo data URL：限定图片 MIME、整串长度和 base64 合法性
fn validate_logo_data_url(logo: &str) -> Result<(), AdminServiceError> {
    if logo.len() > MAX_LOGO_DATA_URL_BYTES {
        return Err(AdminServiceError::InvalidCredential(
            "Logo 不能超过 200 KB".into(),
        ));
    }
    let payload = LOGO_MIME_TYPES.iter().find_map(|mime| {
        logo.strip_prefix("data:")
            .and_then(|rest| rest.strip_prefix(mime))
            .and_then(|rest| rest.strip_prefix(";base64,"))
    });
    match payload {
        Some(payload) if is_valid_base64(payload) => Ok(()),
        Some(_) => Err(AdminServiceError::InvalidCredential(
            "Logo 的 base64 数据无效".into(),
        )),
        None => Err(AdminServiceError::InvalidCredential(
            "Logo 仅支持 SVG、PNG、JPEG、ICO、WebP 格式的 base64 data URL".into(),
        )),
    }
}

/// 标准 base64（带填充）可解码性校验：长度为 4 的倍数、字符集合法、`=` 仅出现在末尾且最多两个
fn is_valid_base64(data: &str) -> bool {
    let bytes = data.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return false;
    }
    let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return false;
    }
    bytes[..bytes.len() - padding]
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
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
            site_name: current.site_name,
            logo_data_url: None,
            clear_logo: None,
            enforce_max_tokens: None,
            input_token_source: None,
            prompt_cache_mode: None,
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

    const PNG_LOGO: &str = "data:image/png;base64,iVBORw0KGgo=";

    fn assert_bad_request(result: Result<CoreConfigResponse, AdminServiceError>) {
        match result {
            Err(error) => assert_eq!(error.status_code(), axum::http::StatusCode::BAD_REQUEST),
            Ok(_) => panic!("应返回 400"),
        }
    }

    #[test]
    fn saves_and_round_trips_site_name_and_logo() {
        let (service, runtime, _manager, _state, _dir) = service();
        let mut request = request(&service);
        request.site_name = Some("  我的站点  ".into());
        request.logo_data_url = Some(PNG_LOGO.into());
        let result = service.update(&admin(), request).unwrap();
        assert_eq!(result.site_name.as_deref(), Some("我的站点"));
        assert_eq!(result.logo_data_url.as_deref(), Some(PNG_LOGO));
        assert_eq!(runtime.get().site_name.as_deref(), Some("我的站点"));
        let persisted = Config::load(runtime.path()).unwrap();
        assert_eq!(persisted.site_name.as_deref(), Some("我的站点"));
        assert_eq!(persisted.logo_data_url.as_deref(), Some(PNG_LOGO));

        // 未传 logo 时保留原值；site_name 传空串清除
        let mut request = super::tests::request(&service);
        request.site_name = Some("   ".into());
        let result = service.update(&admin(), request).unwrap();
        assert_eq!(result.site_name, None);
        assert_eq!(result.logo_data_url.as_deref(), Some(PNG_LOGO));
    }

    #[test]
    fn clear_logo_removes_saved_logo() {
        let (service, runtime, _manager, _state, _dir) = service();
        let mut request = request(&service);
        request.logo_data_url = Some(PNG_LOGO.into());
        service.update(&admin(), request).unwrap();
        let mut request = super::tests::request(&service);
        request.clear_logo = Some(true);
        let result = service.update(&admin(), request).unwrap();
        assert_eq!(result.logo_data_url, None);
        assert_eq!(Config::load(runtime.path()).unwrap().logo_data_url, None);
    }

    #[test]
    fn rejects_invalid_logo_data_url() {
        let (service, runtime, _manager, _state, _dir) = service();
        for logo in [
            "https://example.com/logo.png",
            "data:image/gif;base64,R0lGODlh",
            "data:text/html;base64,PGgxPg==",
            "data:image/png,iVBORw0KGgo=",
            "data:image/png;base64,not*base64",
            "data:image/png;base64,abc",
        ] {
            let mut request = request(&service);
            request.logo_data_url = Some(logo.into());
            assert_bad_request(service.update(&admin(), request));
        }
        assert_eq!(runtime.get().logo_data_url, None);
    }

    #[test]
    fn rejects_logo_over_200_kb() {
        let (service, _runtime, _manager, _state, _dir) = service();
        let prefix = "data:image/png;base64,";
        let body_len = (MAX_LOGO_DATA_URL_BYTES - prefix.len()) / 4 * 4 + 4;
        let mut request = request(&service);
        request.logo_data_url = Some(format!("{prefix}{}", "A".repeat(body_len)));
        assert_bad_request(service.update(&admin(), request));

        let mut request = super::tests::request(&service);
        let body_len = (MAX_LOGO_DATA_URL_BYTES - prefix.len()) / 4 * 4;
        request.logo_data_url = Some(format!("{prefix}{}", "A".repeat(body_len)));
        assert!(service.update(&admin(), request).is_ok());
    }

    #[test]
    fn rejects_site_name_over_60_chars() {
        let (service, _runtime, _manager, _state, _dir) = service();
        let mut request = request(&service);
        request.site_name = Some("站".repeat(61));
        assert_bad_request(service.update(&admin(), request));
        let mut request = super::tests::request(&service);
        request.site_name = Some("站".repeat(60));
        assert!(service.update(&admin(), request).is_ok());
    }

    #[test]
    fn saves_compat_switches_and_keeps_them_when_omitted() {
        let (service, runtime, _manager, _state, _dir) = service();
        // 默认：保持原有行为
        let current = service.get();
        assert!(!current.enforce_max_tokens);
        assert_eq!(current.input_token_source, InputTokenSource::Upstream);
        assert_eq!(current.prompt_cache_mode, PromptCacheMode::Simulated);

        let mut request = request(&service);
        request.enforce_max_tokens = Some(true);
        request.input_token_source = Some(InputTokenSource::Min);
        request.prompt_cache_mode = Some(PromptCacheMode::Disabled);
        let result = service.update(&admin(), request).unwrap();
        assert!(result.enforce_max_tokens);
        assert_eq!(result.input_token_source, InputTokenSource::Min);
        assert_eq!(result.prompt_cache_mode, PromptCacheMode::Disabled);

        // 落盘并热重载
        let persisted = Config::load(runtime.path()).unwrap();
        assert!(persisted.enforce_max_tokens);
        assert_eq!(persisted.input_token_source, InputTokenSource::Min);
        assert_eq!(persisted.prompt_cache_mode, PromptCacheMode::Disabled);
        assert!(runtime.get().enforce_max_tokens);

        // 未传这些字段时保留已保存的值
        let result = service
            .update(&admin(), super::tests::request(&service))
            .unwrap();
        assert!(result.enforce_max_tokens);
        assert_eq!(result.input_token_source, InputTokenSource::Min);
        assert_eq!(result.prompt_cache_mode, PromptCacheMode::Disabled);
    }
}
