//! Atomic local writes and recovery. Backups are never part of the P2P manifest.
use anyhow::{bail, Context};
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
};

type FileLock = Arc<Mutex<()>>;
static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
static NOTICES: OnceLock<Mutex<Vec<RecoveryNotice>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize)]
pub struct RecoveryNotice {
    pub path: String,
    pub recovered: bool,
}

pub fn take_notices() -> Vec<RecoveryNotice> {
    std::mem::take(&mut *NOTICES.get_or_init(Mutex::default).lock())
}

pub(crate) fn report_recovery(path: &Path, recovered: bool) {
    let mut notices = NOTICES.get_or_init(Mutex::default).lock();
    let path = path.to_string_lossy().into_owned();
    if !notices
        .iter()
        .any(|n| n.path == path && n.recovered == recovered)
    {
        notices.push(RecoveryNotice { path, recovered });
    }
}

pub(crate) fn remove_file(path: &Path) -> anyhow::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn lock(path: &Path) -> FileLock {
    let mut locks = LOCKS.get_or_init(Mutex::default).lock();
    if locks.len() > 256 {
        locks.retain(|_, lock| lock.strong_count() > 0);
    }
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

fn replace(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("file has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut pending = tempfile::Builder::new()
        .prefix(".lownotes-write-")
        .tempfile_in(parent)?;
    if let Ok(metadata) = fs::metadata(path) {
        pending.as_file().set_permissions(metadata.permissions())?;
    }
    pending.write_all(bytes)?;
    pending.as_file().sync_all()?;
    // tempfile uses an atomic replacement on Unix and Windows; never unlink the destination first.
    persist_atomic(pending, path)
        .with_context(|| format!("replace local file {}", path.display()))?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn persist_atomic(mut pending: tempfile::NamedTempFile, path: &Path) -> std::io::Result<()> {
    for attempt in 0..=6 {
        match pending.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) => {
                // Windows scanners/editors can briefly hold a handle without
                // FILE_SHARE_DELETE. Keep the same synced temporary file and
                // atomic replacement; never unlink the valid destination.
                let temporary_lock =
                    cfg!(windows) && matches!(error.error.raw_os_error(), Some(5 | 32 | 33));
                if !temporary_lock || attempt == 6 {
                    return Err(error.error);
                }
                pending = error.file;
                std::thread::sleep(std::time::Duration::from_millis(5 << attempt));
            }
        }
    }
    unreachable!("bounded persistence retry returned")
}

pub fn write_validated(
    path: &Path,
    bytes: &[u8],
    valid: impl Fn(&[u8]) -> bool,
) -> anyhow::Result<()> {
    write_with_hook(path, bytes, valid, || Ok(()))
}

/// Used when scrubbing a legacy secret backup: never make another plaintext copy.
pub(crate) fn write_without_backup(
    path: &Path,
    bytes: &[u8],
    valid: impl Fn(&[u8]) -> bool,
) -> anyhow::Result<()> {
    if !valid(bytes) {
        bail!("refusing to persist invalid data");
    }
    let lock = lock(path);
    let _guard = lock.lock();
    replace(path, bytes)
}

fn write_with_hook(
    path: &Path,
    bytes: &[u8],
    valid: impl Fn(&[u8]) -> bool,
    before_commit: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if !valid(bytes) {
        bail!("refusing to persist invalid data");
    }
    let lock = lock(path);
    let _guard = lock.lock();
    let previous = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(previous) = previous {
        if previous == bytes {
            return Ok(());
        }
        if valid(&previous) {
            let modified = fs::metadata(path)?.modified()?;
            let backup = backup_path(path);
            replace(&backup, &previous)?;
            filetime::set_file_mtime(&backup, filetime::FileTime::from_system_time(modified))?;
        } else {
            quarantine(path, &previous)?;
            report_recovery(path, false);
        }
    }
    before_commit()?;
    replace(path, bytes)
}

pub fn write_text(path: &Path, text: &str) -> anyhow::Result<()> {
    write_validated(path, text.as_bytes(), |bytes| {
        std::str::from_utf8(bytes).is_ok()
    })
}

/// Validates before recovering, preserving corrupt bytes for manual recovery.
pub fn read_validated(
    path: &Path,
    valid: impl Fn(&[u8]) -> bool,
) -> anyhow::Result<Option<Vec<u8>>> {
    let result = read_inner(path, valid);
    if result.is_err() {
        report_recovery(path, false);
    }
    result
}

fn read_inner(path: &Path, valid: impl Fn(&[u8]) -> bool) -> anyhow::Result<Option<Vec<u8>>> {
    let lock = lock(path);
    let _guard = lock.lock();
    let primary = match fs::read(path) {
        Ok(bytes) if valid(&bytes) => return Ok(Some(bytes)),
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let backup = backup_path(path);
    if let Ok(bytes) = fs::read(&backup) {
        if valid(&bytes) {
            if let Some(corrupt) = primary.as_ref() {
                quarantine(path, corrupt)?;
            }
            let modified = fs::metadata(&backup)?.modified()?;
            replace(path, &bytes)?;
            filetime::set_file_mtime(path, filetime::FileTime::from_system_time(modified))?;
            report_recovery(path, true);
            return Ok(Some(bytes));
        }
    }
    if primary.is_some() || backup.exists() {
        report_recovery(path, false);
        bail!("storage.corrupt");
    }
    Ok(None)
}

fn quarantine(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("file has no parent directory")?;
    let name = path
        .file_name()
        .context("file has no name")?
        .to_string_lossy();
    let mut archived = tempfile::Builder::new()
        .prefix(&format!(".{name}.corrupt-"))
        .tempfile_in(parent)?;
    archived.write_all(bytes)?;
    archived.as_file().sync_all()?;
    archived.keep().map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json(bytes: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(bytes).is_ok()
    }

    #[test]
    fn failed_commit_keeps_the_primary_and_backup_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_validated(&path, br#"{"value":1}"#, json).unwrap();
        assert!(write_with_hook(&path, br#"{"value":2}"#, json, || bail!(
            "injected interruption"
        ))
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), br#"{"value":1}"#);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), br#"{"value":1}"#);
    }

    #[test]
    fn recovery_retains_corrupt_bytes_and_original_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_validated(&path, br#"{"value":1}"#, json).unwrap();
        write_validated(&path, br#"{"value":2}"#, json).unwrap();
        let modified = fs::metadata(backup_path(&path))
            .unwrap()
            .modified()
            .unwrap();
        fs::write(&path, b"partial").unwrap();
        assert_eq!(
            read_validated(&path, json).unwrap().unwrap(),
            br#"{"value":1}"#
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert!(fs::read_dir(dir.path())
            .unwrap()
            .any(|f| fs::read(f.unwrap().path()).ok().as_deref() == Some(b"partial")));
    }

    #[test]
    fn unrecoverable_data_is_not_overwritten_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"broken").unwrap();
        assert!(read_validated(&path, json).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
        assert!(write_validated(&path, b"bad new data", json).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
    }

    #[test]
    fn identical_writes_do_not_change_file_timestamps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        write_text(&path, "# Note").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        write_text(&path, "# Note").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert!(!backup_path(&path).exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_temporary_windows_delete_lock_preserves_the_old_file_until_atomic_replacement() {
        use std::{os::windows::fs::OpenOptionsExt, sync::mpsc, time::Duration};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_validated(&path, br#"{"value":1}"#, json).unwrap();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        let (ready, started) = mpsc::channel();
        let worker_path = path.clone();
        let worker = std::thread::spawn(move || {
            write_with_hook(&worker_path, br#"{"value":2}"#, json, || {
                ready.send(()).unwrap();
                Ok(())
            })
        });
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(fs::read(&path).unwrap(), br#"{"value":1}"#);
        drop(held);
        worker.join().unwrap().unwrap();
        assert_eq!(fs::read(&path).unwrap(), br#"{"value":2}"#);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), br#"{"value":1}"#);
    }

    #[cfg(windows)]
    #[test]
    fn a_persistent_windows_delete_lock_returns_an_error_without_removing_either_version() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_validated(&path, br#"{"value":1}"#, json).unwrap();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        assert!(write_validated(&path, br#"{"value":2}"#, json).is_err());
        assert_eq!(fs::read(&path).unwrap(), br#"{"value":1}"#);
        assert_eq!(fs::read(backup_path(&path)).unwrap(), br#"{"value":1}"#);
        drop(held);
        write_validated(&path, br#"{"value":2}"#, json).unwrap();
        assert_eq!(fs::read(&path).unwrap(), br#"{"value":2}"#);
    }
}
