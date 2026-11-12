use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::Path;

use anyhow::Context;
use fs2::FileExt;
use crate::kiro::model::credentials::KiroCredentials;

pub fn merge_credential(
    original: Option<&KiroCredentials>,
    local: &KiroCredentials,
    disk: &KiroCredentials,
) -> anyhow::Result<KiroCredentials> {
    let original = serde_json::to_value(original.unwrap_or(disk))?;
    let local = serde_json::to_value(local)?;
    let disk = serde_json::to_value(disk)?;
    let mut merged = disk.clone();
    if let (Some(original), Some(local), Some(merged)) = (original.as_object(), local.as_object(), merged.as_object_mut()) {
        for (field, local_value) in local {
            if original.get(field) != Some(local_value) {
                merged.insert(field.clone(), local_value.clone());
            }
        }
    }
    Ok(serde_json::from_value(merged)?)
}

pub fn merge_credentials(
    local: &[KiroCredentials],
    snapshots: &HashMap<u64, Option<KiroCredentials>>,
    disk: Vec<KiroCredentials>,
) -> anyhow::Result<Vec<KiroCredentials>> {
    let mut merged = Vec::new();
    for current in disk {
        let Some(id) = current.id else { continue };
        let Some(local) = local.iter().find(|cred| cred.id == Some(id)) else {
            if !snapshots.contains_key(&id) {
                merged.push(current);
            }
            continue;
        };
        let original = snapshots.get(&id).and_then(|cred| cred.as_ref());
        let original_refresh = original.and_then(|cred| cred.refresh_token.as_deref());
        let current_refresh = current.refresh_token.as_deref();
        if original_refresh != current_refresh && original_refresh != local.refresh_token.as_deref() {
            merged.push(current);
            continue;
        }
        merged.push(merge_credential(original, local, &current)?);
    }
    for local in local {
        if !merged.iter().any(|cred| cred.id == local.id) {
            merged.push(local.clone());
        }
    }
    let mut next_id = merged.iter().filter_map(|cred| cred.id).max().unwrap_or(0) + 1;
    let mut ids = std::collections::HashSet::new();
    for cred in &mut merged {
        if cred.id.is_none_or(|id| !ids.insert(id)) {
            cred.id = Some(next_id);
            next_id += 1;
            ids.insert(cred.id.unwrap());
        }
    }
    merged.sort_by_key(|cred| (cred.priority, cred.id));
    Ok(merged)
}
pub fn acquire(path: &Path) -> anyhow::Result<FileLock> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("打开共享配置锁失败: {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("等待共享配置锁失败: {}", path.display()))?;
    Ok(FileLock(file))
}

pub struct FileLock(File);

impl Drop for FileLock {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.0) {
            tracing::warn!("释放共享配置锁失败: {}", error);
        }
    }
}

pub fn atomic_write(path: &Path, contents: &str) -> anyhow::Result<()> {
    let parent = path.parent().context("配置文件必须位于目录中")?;
    let name = path.file_name().context("配置文件名无效")?.to_string_lossy();
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, contents)
        .with_context(|| format!("写入临时配置失败: {}", temporary.display()))?;
    let result = (|| {
        File::open(&temporary)?.sync_all()?;
        std::fs::rename(&temporary, path)
            .with_context(|| format!("原子替换配置失败: {}", path.display()))?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_and_released() {
        let path = std::env::temp_dir().join(format!("kiro-shared-{}.lock", uuid::Uuid::new_v4()));
        let lock = acquire(&path).unwrap();
        let mut child = std::process::Command::new("flock")
            .arg("--nonblock")
            .arg("--exclusive")
            .arg(&path)
            .arg("--command")
            .arg("exit 0")
            .spawn()
            .unwrap();
        assert!(!child.wait().unwrap().success());
        drop(lock);
        let second = acquire(&path).unwrap();
        drop(second);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn atomic_write_replaces_contents() {
        let path = std::env::temp_dir().join(format!("kiro-shared-{}.json", uuid::Uuid::new_v4()));
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn merge_preserves_other_instance_token_refresh() {
        let original = KiroCredentials {
            id: Some(1),
            access_token: Some("old-access".into()),
            refresh_token: Some("old-refresh".into()),
            expires_at: Some("old-expiry".into()),
            ..Default::default()
        };
        let mut local = original.clone();
        local.disabled = true;
        let disk = KiroCredentials {
            access_token: Some("new-access".into()),
            refresh_token: Some("new-refresh".into()),
            expires_at: Some("new-expiry".into()),
            profile_arn: Some("new-profile".into()),
            ..original.clone()
        };
        let merged = merge_credential(Some(&original), &local, &disk).unwrap();
        assert!(merged.disabled);
        assert_eq!(merged.access_token.as_deref(), Some("new-access"));
        assert_eq!(merged.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(merged.expires_at.as_deref(), Some("new-expiry"));
        assert_eq!(merged.profile_arn.as_deref(), Some("new-profile"));
    }

    #[test]
    fn merge_applies_local_refresh_over_disk_snapshot() {
        let original = KiroCredentials {
            id: Some(1),
            access_token: Some("old-access".into()),
            refresh_token: Some("old-refresh".into()),
            ..Default::default()
        };
        let mut local = original.clone();
        local.access_token = Some("new-access".into());
        local.refresh_token = Some("new-refresh".into());
        let merged = merge_credential(Some(&original), &local, &original).unwrap();
        assert_eq!(merged.access_token.as_deref(), Some("new-access"));
        assert_eq!(merged.refresh_token.as_deref(), Some("new-refresh"));
    }
}
