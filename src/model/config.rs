use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TlsBackend {
    Rustls,
    NativeTls,
}

impl Default for TlsBackend {
    fn default() -> Self {
        Self::Rustls
    }
}

/// 上报给客户端的输入 token 口径
///
/// 上游 Kiro 只返回上下文占用百分比，乘以窗口大小得到的是整段会话的累计占用，
/// 不是本次请求的输入量，所以同一会话越聊越高。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InputTokenSource {
    /// 上游上下文占用百分比换算值（原有行为）
    #[default]
    Upstream,
    /// 本地分词估算本次请求的输入量
    Local,
    /// 取两者中较小的一个
    Min,
}

/// prompt cache 用量的模拟模式
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PromptCacheMode {
    /// 按本地断点规则模拟缓存读写（原有行为）
    #[default]
    Simulated,
    /// 不模拟，输入全部计为普通输入，缓存读写恒为 0
    Disabled,
}

/// KNA 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_region")]
    pub region: String,

    /// Auth Region（用于 Token 刷新），未配置时回退到 region
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_region: Option<String>,

    /// API Region（用于 API 请求），未配置时回退到 region
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_region: Option<String>,

    #[serde(default = "default_kiro_version")]
    pub kiro_version: String,

    #[serde(default)]
    pub machine_id: Option<String>,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(default = "default_system_version")]
    pub system_version: String,

    #[serde(default = "default_node_version")]
    pub node_version: String,

    #[serde(default = "default_tls_backend")]
    pub tls_backend: TlsBackend,

    /// 外部 count_tokens API 地址（可选）
    #[serde(default)]
    pub count_tokens_api_url: Option<String>,

    /// count_tokens API 密钥（可选）
    #[serde(default)]
    pub count_tokens_api_key: Option<String>,

    /// count_tokens API 认证类型（可选，"x-api-key" 或 "bearer"，默认 "x-api-key"）
    #[serde(default = "default_count_tokens_auth_type")]
    pub count_tokens_auth_type: String,

    /// HTTP 代理地址（可选）
    /// 支持格式: http://host:port, https://host:port, socks5://host:port
    #[serde(default)]
    pub proxy_url: Option<String>,

    /// 代理认证用户名（可选）
    #[serde(default)]
    pub proxy_username: Option<String>,

    /// 代理认证密码（可选）
    #[serde(default)]
    pub proxy_password: Option<String>,

    /// Admin API 密钥（可选，启用 Admin API 功能）
    #[serde(default)]
    pub admin_api_key: Option<String>,

    /// 负载均衡模式（"priority" 或 "balanced"）
    #[serde(default = "default_load_balancing_mode")]
    pub load_balancing_mode: String,

    /// 是否开启非流式响应的 thinking 块提取（默认 true）
    ///
    /// 启用后，非流式响应中的 `<thinking>...</thinking>` 标签会被解析为
    /// 独立的 `{"type": "thinking", ...}` 内容块,与流式响应行为一致。
    #[serde(default = "default_extract_thinking")]
    pub extract_thinking: bool,

    /// 默认端点名称（凭据未显式指定 endpoint 时使用，默认 "ide"）
    #[serde(default = "default_endpoint")]
    pub default_endpoint: String,

    /// 端点特定的配置
    ///
    /// 键为端点名（如 "ide" / "cli"），值为该端点自由定义的参数对象。
    /// 未在此表出现的端点沿用实现内置默认值。
    #[serde(default)]
    pub endpoints: HashMap<String, serde_json::Value>,

    /// 出口代理健康检测地址（GET 后响应体应为纯文本 IP），默认 https://checkip.amazonaws.com
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_check_url: Option<String>,

    /// 站点名称（后台页头、登录页和浏览器标题），未配置时前端显示默认名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,

    /// 站点 Logo（data URL，如 data:image/png;base64,...）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_data_url: Option<String>,

    /// 是否在本地强制执行客户端的 `max_tokens`（默认 false）
    ///
    /// 上游 Kiro 协议没有 `max_tokens` 字段，客户端传的值只用于预扣，不会约束生成。
    /// 开启后，输出 token 累计达到该上限时本地停止转发后续内容，并把
    /// `stop_reason` 置为 `max_tokens`，与 Anthropic 行为一致。
    #[serde(default)]
    pub enforce_max_tokens: bool,

    /// 上报输入 token 的口径（默认 upstream，保持原有行为）
    #[serde(default)]
    pub input_token_source: InputTokenSource,

    /// 模拟 prompt cache 的模式（默认 simulated，保持原有行为）
    #[serde(default)]
    pub prompt_cache_mode: PromptCacheMode,

    /// 配置文件路径（运行时元数据，不写入 JSON）
    #[serde(skip)]
    config_path: Option<PathBuf>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_region() -> String {
    "us-east-1".to_string()
}

fn default_kiro_version() -> String {
    "0.11.133".to_string()
}

fn default_system_version() -> String {
    const SYSTEM_VERSIONS: &[&str] = &["darwin#24.6.0", "win32#10.0.22631"];
    SYSTEM_VERSIONS[fastrand::usize(..SYSTEM_VERSIONS.len())].to_string()
}

fn default_node_version() -> String {
    "22.22.0".to_string()
}

fn default_count_tokens_auth_type() -> String {
    "x-api-key".to_string()
}

fn default_tls_backend() -> TlsBackend {
    TlsBackend::Rustls
}

fn default_load_balancing_mode() -> String {
    "priority".to_string()
}

fn default_extract_thinking() -> bool {
    true
}

fn default_endpoint() -> String {
    crate::kiro::endpoint::ide::IDE_ENDPOINT_NAME.to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            region: default_region(),
            auth_region: None,
            api_region: None,
            kiro_version: default_kiro_version(),
            machine_id: None,
            api_key: None,
            system_version: default_system_version(),
            node_version: default_node_version(),
            tls_backend: default_tls_backend(),
            count_tokens_api_url: None,
            count_tokens_api_key: None,
            count_tokens_auth_type: default_count_tokens_auth_type(),
            proxy_url: None,
            proxy_username: None,
            proxy_password: None,
            admin_api_key: None,
            load_balancing_mode: default_load_balancing_mode(),
            extract_thinking: default_extract_thinking(),
            default_endpoint: default_endpoint(),
            endpoints: HashMap::new(),
            egress_check_url: None,
            site_name: None,
            logo_data_url: None,
            enforce_max_tokens: false,
            input_token_source: InputTokenSource::default(),
            prompt_cache_mode: PromptCacheMode::default(),
            config_path: None,
        }
    }
}

impl Config {
    /// 获取默认配置文件路径
    pub fn default_config_path() -> &'static str {
        "config.json"
    }

    /// 获取有效的 Auth Region（用于 Token 刷新）
    /// 优先使用 auth_region，未配置时回退到 region
    pub fn effective_auth_region(&self) -> &str {
        self.auth_region.as_deref().unwrap_or(&self.region)
    }

    /// 获取有效的 API Region（用于 API 请求）
    /// 优先使用 api_region，未配置时回退到 region
    pub fn effective_api_region(&self) -> &str {
        self.api_region.as_deref().unwrap_or(&self.region)
    }

    /// 获取出口代理检测地址，未配置或为空时使用默认值
    pub fn effective_egress_check_url(&self) -> &str {
        self.egress_check_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .unwrap_or(crate::proxy_pool::DEFAULT_EGRESS_CHECK_URL)
    }

    /// 从文件加载配置
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            // 配置文件不存在，返回默认配置
            let mut config = Self::default();
            config.config_path = Some(path.to_path_buf());
            return Ok(config);
        }

        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        config.config_path = Some(path.to_path_buf());
        Ok(config)
    }

    /// 获取配置文件路径（如果有）
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// 将当前配置写回原始配置文件
    pub fn save(&self) -> anyhow::Result<()> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("配置文件路径未知，无法保存配置"))?;
        let _lock = crate::shared_files::acquire(&path.with_extension("lock"))?;
        self.save_locked()
    }

    pub fn save_locked(&self) -> anyhow::Result<()> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("配置文件路径未知，无法保存配置"))?;
        let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        crate::shared_files::atomic_write(path, &content)
            .with_context(|| format!("写入配置文件失败: {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod compat_serde_tests {
    use super::*;

    #[test]
    fn compat_switches_serialize_with_expected_names() {
        let config = Config {
            enforce_max_tokens: true,
            input_token_source: InputTokenSource::Min,
            prompt_cache_mode: PromptCacheMode::Disabled,
            ..Default::default()
        };
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["enforceMaxTokens"], true);
        assert_eq!(json["inputTokenSource"], "min");
        assert_eq!(json["promptCacheMode"], "disabled");

        // 旧配置文件没有这些字段时取默认值，保持原有行为
        let legacy: Config = serde_json::from_str(r#"{"region":"us-east-1"}"#).unwrap();
        assert!(!legacy.enforce_max_tokens);
        assert_eq!(legacy.input_token_source, InputTokenSource::Upstream);
        assert_eq!(legacy.prompt_cache_mode, PromptCacheMode::Simulated);

        // kebab-case 枚举值可回读
        let round: Config = serde_json::from_value(json).unwrap();
        assert_eq!(round.input_token_source, InputTokenSource::Min);
        assert_eq!(round.prompt_cache_mode, PromptCacheMode::Disabled);
    }
}
