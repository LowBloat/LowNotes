//! A durable intent coordinates the Markdown file and its collaborative state.
use crate::{crdt::CrdtManager, storage, vault};
use anyhow::bail;
use base64::{engine::general_purpose::STANDARD, Engine};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use yrs::{updates::decoder::Decode, Doc, GetString, Transact, Update};

const DIRECTORY: &str = ".lownotes/pending-notes";
static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Serialize, Deserialize)]
struct Intent {
    version: u8,
    path: String,
    content: String,
    state: String,
    previous_hash: Option<String>,
}

fn path_for(root: &Path, note: &str) -> PathBuf {
    root.join(DIRECTORY)
        .join(format!("{}.json", blake3::hash(note.as_bytes()).to_hex()))
}

fn parse(bytes: &[u8], file: &Path) -> anyhow::Result<Intent> {
    let intent: Intent = serde_json::from_slice(bytes)?;
    if intent.version != 1
        || !vault::is_markdown(Path::new(&intent.path))
        || intent.content.len() as u64 > vault::MAX_NOTE_BYTES
        || file.file_name() != path_for(Path::new(""), &intent.path).file_name()
    {
        bail!("invalid note transaction");
    }
    let doc = Doc::new();
    doc.transact_mut()
        .apply_update(Update::decode_v1(&STANDARD.decode(&intent.state)?)?)?;
    let text = doc
        .get_or_insert_text("content")
        .get_string(&doc.transact());
    if text != intent.content {
        bail!("note transaction does not match its collaborative state");
    }
    Ok(intent)
}

fn finish(root: &Path, file: &Path, intent: &Intent, recovered: bool) -> anyhow::Result<()> {
    let target = vault::safe_join(root, &intent.path)?;
    // An external editor may have saved after the crash. Keep that version too.
    if recovered {
        if let Ok(current) = fs::read_to_string(&target) {
            let hash = blake3::hash(current.as_bytes()).to_hex().to_string();
            if current != intent.content && Some(&hash) != intent.previous_hash.as_ref() {
                let copy = CrdtManager::conflict_path(&intent.path, &current)?;
                let copy_path = vault::safe_join(root, &copy)?;
                if copy_path.exists() && fs::read_to_string(&copy_path)? != current {
                    bail!("recovery copy already contains different text");
                }
                storage::write_text(&copy_path, &current)?;
                storage::report_recovery(&copy_path, true);
            }
        }
    }
    CrdtManager::write_state(root, &intent.path, &STANDARD.decode(&intent.state)?)?;
    storage::write_text(&target, &intent.content)?;
    // Remove backup first: a crash during cleanup must not replay an older intent.
    storage::remove_file(&storage::backup_path(file))?;
    storage::remove_file(file)?;
    if recovered {
        storage::report_recovery(&target, true);
    }
    Ok(())
}

fn recover_file(root: &Path, file: &Path) -> anyhow::Result<bool> {
    let Some(bytes) = storage::read_validated(file, |bytes| parse(bytes, file).is_ok())? else {
        return Ok(false);
    };
    let intent = parse(&bytes, file)?;
    finish(root, file, &intent, true)?;
    Ok(true)
}

pub fn recover_note(root: &Path, note: &str) -> anyhow::Result<bool> {
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    recover_file(root, &path_for(root, note))
}

pub fn recover_all(root: &Path) -> anyhow::Result<()> {
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    let directory = root.join(DIRECTORY);
    if !directory.is_dir() {
        return Ok(());
    }
    let mut files = std::collections::BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") {
            files.insert(entry.path());
        } else if name.ends_with(".json.bak") {
            files.insert(entry.path().with_file_name(name.trim_end_matches(".bak")));
        }
    }
    for file in files {
        recover_file(root, &file)?;
    }
    Ok(())
}

pub fn commit(root: &Path, note: &str, content: &str, state: &[u8]) -> anyhow::Result<()> {
    commit_with_hook(root, note, content, state, |_| Ok(()))
}

pub(crate) fn commit_with_hook(
    root: &Path,
    note: &str,
    content: &str,
    state: &[u8],
    hook: impl Fn(u8) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let _guard = LOCK.get_or_init(Mutex::default).lock();
    let file = path_for(root, note);
    recover_file(root, &file)?;
    let target = vault::safe_join(root, note)?;
    let previous_hash = match fs::read(&target) {
        Ok(bytes) => Some(blake3::hash(&bytes).to_hex().to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let intent = Intent {
        version: 1,
        path: note.into(),
        content: content.into(),
        state: STANDARD.encode(state),
        previous_hash,
    };
    let bytes = serde_json::to_vec(&intent)?;
    storage::write_validated(&file, &bytes, |bytes| parse(bytes, &file).is_ok())?;
    hook(0)?;
    CrdtManager::write_state(root, note, state)?;
    hook(1)?;
    storage::write_text(&target, content)?;
    hook(2)?;
    storage::remove_file(&storage::backup_path(&file))?;
    storage::remove_file(&file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::Text;
    fn state(content: &str) -> Vec<u8> {
        let doc = Doc::new();
        doc.get_or_insert_text("content")
            .push(&mut doc.transact_mut(), content);
        CrdtManager::encode_state(&doc)
    }

    #[test]
    fn restart_completes_each_interrupted_phase_without_losing_either_file() {
        for phase in 0..3 {
            let root = tempfile::tempdir().unwrap();
            vault::save_note(root.path(), "note.md", "old text").unwrap();
            assert!(commit_with_hook(
                root.path(),
                "note.md",
                "new 🙂 text",
                &state("new 🙂 text"),
                |at| {
                    if at == phase {
                        bail!("simulated process interruption");
                    }
                    Ok(())
                }
            )
            .is_err());
            recover_all(root.path()).unwrap();
            assert_eq!(
                vault::read_note(root.path(), "note.md").unwrap(),
                "new 🙂 text"
            );
            let manager = CrdtManager::new();
            manager.get_or_create_doc(root.path(), "note.md").unwrap();
            assert_eq!(
                vault::read_note(root.path(), "note.md").unwrap(),
                "new 🙂 text"
            );
            assert!(!path_for(root.path(), "note.md").exists());
            recover_all(root.path()).unwrap();
        }
    }

    #[test]
    fn recovery_preserves_a_new_external_edit_as_a_review_copy() {
        let root = tempfile::tempdir().unwrap();
        vault::save_note(root.path(), "note.md", "original").unwrap();
        assert!(
            commit_with_hook(root.path(), "note.md", "saved", &state("saved"), |at| {
                if at == 1 {
                    bail!("interrupted");
                }
                Ok(())
            })
            .is_err()
        );
        fs::write(root.path().join("note.md"), "external change").unwrap();
        recover_all(root.path()).unwrap();
        let copy = CrdtManager::conflict_path("note.md", "external change").unwrap();
        assert_eq!(
            vault::read_note(root.path(), &copy).unwrap(),
            "external change"
        );
        assert_eq!(vault::read_note(root.path(), "note.md").unwrap(), "saved");
    }

    #[test]
    fn invalid_intent_cannot_change_markdown_or_collaborative_state() {
        let root = tempfile::tempdir().unwrap();
        vault::save_note(root.path(), "note.md", "original").unwrap();
        let file = path_for(root.path(), "note.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"truncated").unwrap();
        assert!(recover_all(root.path()).is_err());
        assert_eq!(
            vault::read_note(root.path(), "note.md").unwrap(),
            "original"
        );
        assert_eq!(fs::read(&file).unwrap(), b"truncated");
    }

    #[test]
    fn abrupt_process_exit_recovers_both_markdown_and_collaborative_state() {
        for phase in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "note_transaction::tests::crash_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("LOWNOTES_TEST_CRASH_ROOT", root.path())
                .env("LOWNOTES_TEST_CRASH_PHASE", phase.to_string())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(86),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let manager = CrdtManager::new();
            let bytes = manager.get_or_create_doc(root.path(), "note.md").unwrap();
            let doc = Doc::new();
            doc.transact_mut()
                .apply_update(Update::decode_v1(&bytes).unwrap())
                .unwrap();
            assert_eq!(
                doc.get_or_insert_text("content")
                    .get_string(&doc.transact()),
                "committed 🙂 edit"
            );
            assert_eq!(
                vault::read_note(root.path(), "note.md").unwrap(),
                "committed 🙂 edit"
            );
            assert!(!path_for(root.path(), "note.md").exists());
        }
    }

    #[test]
    #[ignore = "Isolated child process for the abrupt-exit regression test"]
    fn crash_worker() {
        let root = PathBuf::from(
            std::env::var_os("LOWNOTES_TEST_CRASH_ROOT").expect("isolated fixture directory"),
        );
        let phase: u8 = std::env::var("LOWNOTES_TEST_CRASH_PHASE")
            .unwrap()
            .parse()
            .unwrap();
        vault::save_note(&root, "note.md", "original").unwrap();
        commit_with_hook(
            &root,
            "note.md",
            "committed 🙂 edit",
            &state("committed 🙂 edit"),
            |at| {
                if at == phase {
                    std::process::exit(86);
                }
                Ok(())
            },
        )
        .unwrap();
        panic!("child process did not stop at the requested phase");
    }
}
