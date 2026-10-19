//! Diagnostic log for troubleshooting sync, storage and updates.
//!
//! Written to `<app data>/logs/lownotes.log` as plain text, rotated at 2 MiB
//! with three older files kept. It records errors and file names, never note
//! content, keys, pairing codes or prompts. Set `LOWNOTES_LOG` (for example
//! `LOWNOTES_LOG=debug`) to change the level.

use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
};

use directories::ProjectDirs;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const KEEP: usize = 3;

pub fn log_dir() -> Option<PathBuf> {
    Some(ProjectDirs::from("dev", "lowbloat", "lownotes")?.data_local_dir().join("logs"))
}

/// Shifts `lownotes.log` to `.1`, `.1` to `.2`, ... when it grew too large.
fn rotate(path: &Path) {
    if fs::metadata(path).map(|meta| meta.len() < MAX_BYTES).unwrap_or(true) {
        return;
    }
    let numbered = |n: usize| path.with_extension(format!("log.{n}"));
    let _ = fs::remove_file(numbered(KEEP));
    for n in (1..KEEP).rev() {
        let _ = fs::rename(numbered(n), numbered(n + 1));
    }
    let _ = fs::rename(path, numbered(1));
}

/// Installs the global subscriber. Safe to call once; logging failures never
/// stop the app.
pub fn init() {
    let filter = EnvFilter::try_from_env("LOWNOTES_LOG").unwrap_or_else(|_| EnvFilter::new("warn,lownotes_lib=info"));
    let file = log_dir().and_then(|dir| {
        fs::create_dir_all(&dir).ok()?;
        let path = dir.join("lownotes.log");
        rotate(&path);
        OpenOptions::new().create(true).append(true).open(path).ok()
    });
    let file_layer = file.map(|file| fmt::layer().with_writer(Mutex::new(file)).with_target(true));
    let console_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));
    let _ = tracing_subscriber::registry().with(filter).with(file_layer).with(console_layer).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_keeps_a_bounded_number_of_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lownotes.log");
        for round in 0..5 {
            fs::write(&path, vec![b'x'; MAX_BYTES as usize + round]).unwrap();
            rotate(&path);
            assert!(!path.exists());
        }
        let mut names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["lownotes.log.1", "lownotes.log.2", "lownotes.log.3"]);
        // A small file is left alone.
        fs::write(&path, b"small").unwrap();
        rotate(&path);
        assert!(path.exists());
    }
}
