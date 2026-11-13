use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::http_client::ProxyConfig;
use crate::model::config::Config;

pub struct RuntimeConfig {
    current: Mutex<Arc<Config>>,
    path: PathBuf,
    revision: std::sync::atomic::AtomicU64,
}

impl RuntimeConfig {
    pub fn new(config: Config, path: impl Into<PathBuf>) -> Self {
        Self {
            current: Mutex::new(Arc::new(config)),
            path: path.into(),
            revision: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn from_config(config: Config) -> Arc<Self> {
        let path = config
            .config_path()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(Config::default_config_path()));
        Arc::new(Self::new(config, path))
    }

    pub fn get(&self) -> Arc<Config> {
        self.current.lock().expect("运行时配置锁不可用").clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn replace(&self, config: Config) {
        *self.current.lock().expect("运行时配置锁不可用") = Arc::new(config);
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl From<Config> for Arc<RuntimeConfig> {
    fn from(config: Config) -> Self {
        RuntimeConfig::from_config(config)
    }
}

pub fn proxy_from_config(config: &Config) -> Option<ProxyConfig> {
    config.proxy_url.as_ref().map(|url| {
        let mut proxy = ProxyConfig::new(url);
        if let (Some(username), Some(password)) = (&config.proxy_username, &config.proxy_password) {
            proxy = proxy.with_auth(username, password);
        }
        proxy
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_do_not_change_for_existing_readers() {
        let runtime = RuntimeConfig::new(Config::default(), "/tmp/config.json");
        let before = runtime.get();
        let mut next = Config::default();
        next.kiro_version = "next".into();
        runtime.replace(next);
        assert_eq!(before.kiro_version, Config::default().kiro_version);
        assert_eq!(runtime.get().kiro_version, "next");
    }
}
