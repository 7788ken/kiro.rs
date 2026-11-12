mod accounts;
mod admin;
mod admin_ui;
mod anthropic;
mod common;
mod http_client;
mod kiro;
mod model;
pub mod token;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use accounts::{AccountState, AccountStore};
use axum::{Router, middleware};
use clap::Parser;
use kiro::endpoint::{IdeEndpoint, KiroEndpoint};
use kiro::model::credentials::{CredentialsConfig, KiroCredentials};
use kiro::provider::KiroProvider;
use kiro::token_manager::MultiTokenManager;
use model::arg::Args;
use model::config::Config;

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = args
        .config
        .unwrap_or_else(|| Config::default_config_path().to_string());
    let config = Config::load(&config_path).unwrap_or_else(|e| {
        tracing::error!("加载配置失败: {}", e);
        std::process::exit(1);
    });

    let credentials_path = args
        .credentials
        .unwrap_or_else(|| KiroCredentials::default_credentials_path().to_string());
    let credentials_config = CredentialsConfig::load(&credentials_path).unwrap_or_else(|e| {
        tracing::error!("加载凭证失败: {}", e);
        std::process::exit(1);
    });
    let is_multiple_format = credentials_config.is_multiple();
    let mut credentials_list = credentials_config.into_sorted_credentials();

    if let Ok(kiro_api_key) = std::env::var("KIRO_API_KEY") {
        if kiro_api_key.is_empty() {
            tracing::warn!("KIRO_API_KEY 环境变量已设置但为空，视为未配置");
        } else {
            tracing::info!("检测到 KIRO_API_KEY 环境变量，添加 API Key 凭据（最高优先级）");
            credentials_list.insert(
                0,
                KiroCredentials {
                    kiro_api_key: Some(kiro_api_key),
                    auth_method: Some("api_key".to_string()),
                    priority: 0,
                    ..Default::default()
                },
            );
        }
    }

    tracing::info!("已加载 {} 个凭据配置", credentials_list.len());

    let legacy_api_key = config.api_key.clone().unwrap_or_else(|| {
        tracing::error!("配置文件中未设置 apiKey");
        std::process::exit(1);
    });
    let admin_key = config
        .admin_api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
        .unwrap_or_else(|| {
            tracing::error!("用户管理和积分计费需要配置非空 adminApiKey");
            std::process::exit(1);
        });
    let db_path = account_db_path(&config_path);
    #[cfg(feature = "mysql")]
    let database_url = std::env::var("MYSQL_URL").unwrap_or_else(|_| {
        tracing::error!("MySQL 模式需要配置 MYSQL_URL 环境变量");
        std::process::exit(1);
    });
    #[cfg(feature = "mysql")]
    let account_store = AccountStore::open(&database_url).unwrap_or_else(|error| {
        tracing::error!("连接 MySQL 账户数据库失败: {}", error);
        std::process::exit(1);
    });
    #[cfg(not(feature = "mysql"))]
    let account_store = AccountStore::open(&db_path).unwrap_or_else(|error| {
        tracing::error!("打开账户数据库 {} 失败: {}", db_path.display(), error);
        std::process::exit(1);
    });
    account_store
        .ensure_super_admin(admin_key)
        .unwrap_or_else(|error| {
            tracing::error!("初始化超级管理员失败: {}", error);
            std::process::exit(1);
        });
    let account_state =
        AccountState::new(Arc::new(account_store), admin_key).with_legacy_api_key(legacy_api_key);
    #[cfg(feature = "mysql")]
    tracing::info!("用户、权限及积分账本已启用: MySQL");
    #[cfg(not(feature = "mysql"))]
    tracing::info!("用户、权限及积分账本已启用: {}", db_path.display());

    let proxy_config = config.proxy_url.as_ref().map(|url| {
        let mut proxy = http_client::ProxyConfig::new(url);
        if let (Some(username), Some(password)) = (&config.proxy_username, &config.proxy_password) {
            proxy = proxy.with_auth(username, password);
        }
        proxy
    });

    let mut endpoints: HashMap<String, Arc<dyn KiroEndpoint>> = HashMap::new();
    let ide = IdeEndpoint::new();
    endpoints.insert(ide.name().to_string(), Arc::new(ide));

    if !endpoints.contains_key(&config.default_endpoint) {
        tracing::error!("默认端点 \"{}\" 未注册", config.default_endpoint);
        std::process::exit(1);
    }
    for cred in &credentials_list {
        let name = cred.endpoint.as_deref().unwrap_or(&config.default_endpoint);
        if !endpoints.contains_key(name) {
            tracing::error!("凭据 id={:?} 指定了未知端点 \"{}\"", cred.id, name);
            std::process::exit(1);
        }
    }
    let endpoint_names: Vec<String> = endpoints.keys().cloned().collect();

    let token_manager = MultiTokenManager::new(
        config.clone(),
        credentials_list,
        proxy_config.clone(),
        Some(credentials_path.into()),
        is_multiple_format,
    )
    .unwrap_or_else(|e| {
        tracing::error!("创建 Token 管理器失败: {}", e);
        std::process::exit(1);
    });
    let token_manager = Arc::new(token_manager);
    let kiro_provider = KiroProvider::with_proxy(
        token_manager.clone(),
        proxy_config.clone(),
        endpoints,
        config.default_endpoint.clone(),
    );

    token::init_config(token::CountTokensConfig {
        api_url: config.count_tokens_api_url.clone(),
        api_key: config.count_tokens_api_key.clone(),
        auth_type: config.count_tokens_auth_type.clone(),
        proxy: proxy_config,
        tls_backend: config.tls_backend,
    });

    let anthropic_app = anthropic::create_router_with_accounts(
        Some(kiro_provider),
        config.extract_thinking,
        account_state.clone(),
    );
    let admin_service = admin::AdminService::new(token_manager.clone(), endpoint_names.clone());
    let admin_state = admin::AdminState::new(admin_key, admin_service);
    let admin_app = admin::create_admin_router(admin_state).layer(middleware::from_fn_with_state(
        account_state.clone(),
        accounts::middleware::super_admin_auth_middleware,
    ));

    let app = Router::new()
        .merge(anthropic_app)
        .nest(
            "/api/account",
            accounts::login_router(account_state.clone()),
        )
        .nest(
            "/api/account",
            accounts::account_router(account_state.clone()),
        )
        .nest(
            "/api/admin",
            accounts::admin_account_router(account_state.clone())
                .merge(admin_app)
                .nest("/account", accounts::account_router(account_state.clone())),
        )
        .route(
            "/admin/",
            axum::routing::get(|| async { axum::response::Redirect::permanent("/admin") }),
        )
        .nest("/admin", admin_ui::create_admin_ui_router());

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("启动 Anthropic API 端点: {}", addr);
    tracing::info!("管理控制台: /admin；用户中心: /api/account");
    tracing::info!("API 端点: /v1/messages、/cc/v1/messages");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

fn account_db_path(config_path: &str) -> PathBuf {
    Path::new(config_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("accounts.db")
}
