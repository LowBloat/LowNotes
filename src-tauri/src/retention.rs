//! Per-vault local recovery retention. The default never removes recovery data.
use crate::{crdt::CrdtManager, storage, structural};
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const FILE: &str = ".lownotes/history-settings.json";
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub versions_days: Option<u32>,
    pub trash_days: Option<u32>,
}
#[derive(Serialize, Deserialize)]
struct Stored {
    version: u8,
    policy: RetentionPolicy,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct CleanupReport {
    pub versions: usize,
    pub archives: usize,
    pub protected: usize,
}
impl RetentionPolicy {
    fn validate(&self) -> anyhow::Result<()> {
        if [self.versions_days, self.trash_days]
            .into_iter()
            .flatten()
            .any(|days| days == 0 || days > 36_500)
        {
            bail!("history.invalidRetention");
        }
        Ok(())
    }
}
fn parse(bytes: &[u8]) -> anyhow::Result<Stored> {
    if bytes.len() > 4096 {
        bail!("invalid retention settings");
    }
    let stored: Stored = serde_json::from_slice(bytes)?;
    if stored.version != 1 {
        bail!("invalid retention settings");
    }
    stored.policy.validate()?;
    Ok(stored)
}
pub fn load(root: &Path) -> anyhow::Result<RetentionPolicy> {
    storage::read_validated(&root.join(FILE), |bytes| parse(bytes).is_ok())?
        .map(|bytes| parse(&bytes).map(|stored| stored.policy))
        .transpose()
        .map(|policy| policy.unwrap_or_default())
}
pub fn save(root: &Path, policy: RetentionPolicy) -> anyhow::Result<()> {
    policy.validate()?;
    storage::write_validated(
        &root.join(FILE),
        &serde_json::to_vec(&Stored { version: 1, policy })?,
        |bytes| parse(bytes).is_ok(),
    )
}
pub(crate) fn cutoff(now: u64, days: Option<u32>) -> Option<u64> {
    days.map(|days| now.saturating_sub(days as u64 * 86_400_000))
}
fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
/// Never follow a link/junction or recursively delete beyond the chosen vault.
pub(crate) fn owned_path(root: &Path, target: &Path, tree: bool) -> anyhow::Result<bool> {
    let relative = target
        .strip_prefix(root)
        .context("recovery path is outside the vault")?;
    let canonical_root = root.canonicalize()?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Ok(false);
        }
        path.push(component);
        if is_link(&fs::symlink_metadata(&path)?) {
            return Ok(false);
        }
    }
    if !target.canonicalize()?.starts_with(&canonical_root) {
        return Ok(false);
    }
    if tree {
        for entry in walkdir::WalkDir::new(target).follow_links(false) {
            if is_link(&fs::symlink_metadata(entry?.path())?) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
pub fn cleanup(
    root: &Path,
    peers: &[String],
    manager: &CrdtManager,
    apply: bool,
) -> anyhow::Result<CleanupReport> {
    cleanup_at(
        root,
        peers,
        manager,
        apply,
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64,
    )
}
pub(crate) fn cleanup_at(
    root: &Path,
    peers: &[String],
    manager: &CrdtManager,
    apply: bool,
    now: u64,
) -> anyhow::Result<CleanupReport> {
    let policy = load(root)?;
    if policy == RetentionPolicy::default() {
        return Ok(CleanupReport::default());
    }
    structural::exclusive(root, manager, || {
        let mut report = CleanupReport::default();
        crate::note_history::expire(root, cutoff(now, policy.versions_days), apply, &mut report)?;
        crate::catalog_sync::expire_trash(
            root,
            cutoff(now, policy.trash_days),
            peers,
            apply,
            &mut report,
        )?;
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_defaults_to_forever_and_survives_restart_without_entering_the_manifest() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(load(root.path()).unwrap(), RetentionPolicy::default());
        let policy = RetentionPolicy {
            versions_days: Some(30),
            trash_days: Some(90),
        };
        save(root.path(), policy.clone()).unwrap();
        assert_eq!(load(root.path()).unwrap(), policy);
        assert!(save(
            root.path(),
            RetentionPolicy {
                versions_days: Some(0),
                trash_days: None
            }
        )
        .is_err());
        assert!(crate::vault::build_manifest(root.path())
            .unwrap()
            .is_empty());
    }
    #[test]
    fn cleanup_requires_expiry_and_peer_acknowledgement_and_never_erases_tombstones() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("original"), &manager, "a").unwrap();
        manager
            .replace_note_text(root.path(), "note.md", "changed")
            .unwrap();
        crate::catalog_sync::delete(root.path(), "note.md", &manager, "a").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let future = now + 2 * 86_400_000;
        let peers = vec!["offline-peer".to_string()];
        let catalog = crate::catalog::load(root.path()).unwrap();
        let note_id = catalog
            .resolve()
            .unwrap()
            .into_values()
            .find(|entry| entry.path == "note.md")
            .unwrap()
            .id;
        let forever = cleanup_at(root.path(), &peers, &manager, true, future).unwrap();
        assert_eq!(forever.archives + forever.versions, 0);
        save(
            root.path(),
            RetentionPolicy {
                versions_days: Some(1),
                trash_days: Some(1),
            },
        )
        .unwrap();
        let fresh = cleanup_at(root.path(), &peers, &manager, false, now).unwrap();
        assert_eq!(fresh.archives + fresh.versions, 0);
        let preview = cleanup_at(root.path(), &peers, &manager, false, future).unwrap();
        assert_eq!(preview.versions, 1);
        assert!(preview.protected > 0);
        assert_eq!(
            crate::note_history::list(root.path(), &note_id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            crate::catalog_sync::list_trash(root.path()).unwrap().len(),
            1
        );
        crate::catalog::transact(root.path(), |catalog| {
            catalog.acknowledge(
                "offline-peer",
                catalog.operations.keys().cloned().collect::<Vec<_>>(),
            )?;
            Ok(())
        })
        .unwrap();
        let preview = cleanup_at(root.path(), &peers, &manager, false, future).unwrap();
        assert!(preview.archives > 0);
        assert_eq!(preview.protected, 0);
        let removed = cleanup_at(root.path(), &peers, &manager, true, future).unwrap();
        assert_eq!(removed.archives, preview.archives);
        assert_eq!(removed.versions, 1);
        assert!(crate::note_history::list(root.path(), &note_id)
            .unwrap()
            .is_empty());
        assert!(crate::catalog_sync::list_trash(root.path())
            .unwrap()
            .is_empty());
        let after = crate::catalog::load(root.path()).unwrap();
        assert!(after.resolve().unwrap()[&note_id].deleted());
        assert_eq!(after.operations, catalog.operations);
    }
    #[test]
    fn cleanup_preserves_external_recovery_files() {
        let root = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("original"), &manager, "a").unwrap();
        crate::catalog_sync::delete(root.path(), "note.md", &manager, "a").unwrap();
        let record = crate::catalog_sync::list_trash(root.path())
            .unwrap()
            .remove(0);
        let directory = root
            .path()
            .join(crate::catalog_sync::TRASH)
            .join(record.record_id);
        fs::create_dir_all(directory.join("external")).unwrap();
        fs::write(directory.join("external/important.bin"), "preserve me").unwrap();
        save(
            root.path(),
            RetentionPolicy {
                versions_days: None,
                trash_days: Some(1),
            },
        )
        .unwrap();
        let report = cleanup_at(root.path(), &[], &manager, true, u64::MAX).unwrap();
        assert!(report.protected > 0);
        assert_eq!(
            fs::read_to_string(directory.join("external/important.bin")).unwrap(),
            "preserve me"
        );
    }
    #[test]
    fn cleanup_never_follows_a_directory_link_or_junction() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let manager = CrdtManager::new();
        crate::creation::create(root.path(), "note.md", Some("original"), &manager, "a").unwrap();
        crate::catalog_sync::delete(root.path(), "note.md", &manager, "a").unwrap();
        let record = crate::catalog_sync::list_trash(root.path())
            .unwrap()
            .remove(0);
        let directory = root
            .path()
            .join(crate::catalog_sync::TRASH)
            .join(record.record_id);
        let link = directory.join("unreviewed-link");
        fs::write(outside.path().join("important.bin"), "preserve me").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(outside.path().to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        save(
            root.path(),
            RetentionPolicy {
                versions_days: None,
                trash_days: Some(1),
            },
        )
        .unwrap();
        let report = cleanup_at(root.path(), &[], &manager, true, u64::MAX).unwrap();
        assert!(report.protected > 0);
        assert!(directory.exists());
        assert_eq!(
            fs::read_to_string(outside.path().join("important.bin")).unwrap(),
            "preserve me"
        );
    }
}
