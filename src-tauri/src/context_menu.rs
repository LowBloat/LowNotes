//! "Open with LowNotes" in the file manager, and opening the folder or note
//! LowNotes was launched with.
//!
//! The entries are per user and off by default. Windows gets a context menu
//! entry for folders and Markdown files; Linux gets a hidden desktop entry that
//! file managers list under "Open With". macOS builds Finder's list from the
//! app bundle, which cannot change at run time, so the preference is hidden there.

use crate::config::VaultConfig;
use parking_lot::Mutex;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const SUPPORTED: bool = cfg!(any(windows, target_os = "linux"));

/// Entry label in the interface language. An empty or unknown code falls back
/// to English, as the tray menu does.
pub fn label(language: &str) -> &'static str {
    match language {
        "pt-BR" => "Abrir com LowNotes",
        "es-ES" => "Abrir con LowNotes",
        _ => "Open with LowNotes",
    }
}

/// Adds or removes the entries. Adding again refreshes the label and the
/// program path, which changes when a portable copy moves.
pub fn apply(enabled: bool, language: &str) -> anyhow::Result<()> {
    platform::apply(enabled, label(language))
}

/// The program the entries start: the AppImage file rather than its temporary
/// mount, otherwise the running executable.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn launcher() -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
    {
        return Ok(appimage);
    }
    Ok(std::env::current_exe()?)
}

#[cfg(windows)]
mod platform {
    use windows_registry::CURRENT_USER;

    /// Keys under HKEY_CURRENT_USER with the placeholder Explorer replaces by
    /// the clicked item: a folder, the folder whose background was clicked, or
    /// a Markdown file.
    const ENTRIES: [(&str, &str); 4] = [
        (r"Software\Classes\Directory\shell\LowNotes", "%1"),
        (
            r"Software\Classes\Directory\Background\shell\LowNotes",
            "%V",
        ),
        (
            r"Software\Classes\SystemFileAssociations\.md\shell\LowNotes",
            "%1",
        ),
        (
            r"Software\Classes\SystemFileAssociations\.markdown\shell\LowNotes",
            "%1",
        ),
    ];
    const NOT_FOUND: i32 = 0x8007_0002_u32 as i32;

    pub fn apply(enabled: bool, label: &str) -> anyhow::Result<()> {
        if !enabled {
            for (key, _) in ENTRIES {
                if let Err(error) = CURRENT_USER.remove_tree(key) {
                    if error.code().0 != NOT_FOUND {
                        return Err(error.into());
                    }
                }
            }
            return Ok(());
        }
        let program = super::launcher()?;
        let program = program.to_string_lossy();
        for (key, placeholder) in ENTRIES {
            let entry = CURRENT_USER.create(key)?;
            entry.set_string("", label)?;
            entry.set_string("Icon", &*program)?;
            entry
                .create("command")?
                .set_string("", format!("\"{program}\" \"{placeholder}\""))?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::{Path, PathBuf};

    const FILE_NAME: &str = "lownotes-open-with.desktop";

    pub fn apply(enabled: bool, label: &str) -> anyhow::Result<()> {
        let directory = applications_dir()?;
        let file = directory.join(FILE_NAME);
        if !enabled {
            crate::storage::remove_file(&file)?;
        } else {
            let entry = super::desktop_entry(&super::launcher()?, label);
            if std::fs::read_to_string(&file).ok().as_deref() == Some(entry.as_str()) {
                return Ok(());
            }
            // A .bak copy next to it would be one more file for desktops to parse.
            crate::storage::write_without_backup(&file, entry.as_bytes(), |_| true)?;
        }
        refresh_cache(&directory);
        Ok(())
    }

    fn applications_dir() -> anyhow::Result<PathBuf> {
        let dirs =
            directories::BaseDirs::new().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
        Ok(dirs.data_dir().join("applications"))
    }

    /// Desktops that read mimeinfo.cache only see the entry once the cache is
    /// rebuilt. The tool is optional; without it, file managers scan the folder.
    fn refresh_cache(directory: &Path) {
        let _ = std::process::Command::new("update-desktop-database")
            .arg(directory)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub fn apply(_enabled: bool, _label: &str) -> anyhow::Result<()> {
        anyhow::bail!("unsupported platform")
    }
}

/// A hidden desktop entry offering LowNotes for folders and Markdown files.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(program: &Path, label: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=LowNotes\nComment={label}\nExec={} %f\nIcon=lownotes\nTerminal=false\nNoDisplay=true\nMimeType=inode/directory;text/markdown;text/x-markdown;\n",
        exec_argument(&program.to_string_lossy())
    )
}

/// Quotes a program path for `Exec=`: the Desktop Entry specification escapes
/// `"`, `` ` ``, `$` and `\` inside quotes, doubles `%`, and then escapes every
/// backslash again as a string value.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn exec_argument(path: &str) -> String {
    let mut quoted = String::from("\"");
    for character in path.chars() {
        match character {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(character);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\")
}

/// A folder or note to show, from a launch argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenRequest {
    pub vault_path: String,
    /// Relative to the vault with `/` separators.
    pub note_path: Option<String>,
    /// False when the folder is not in the vault list yet, so the interface
    /// asks before turning it into a vault.
    pub known_vault: bool,
}

/// The latest launch target, kept until the interface takes it.
#[derive(Default)]
pub struct PendingOpen(Mutex<Option<PathBuf>>);

impl PendingOpen {
    pub fn set(&self, path: PathBuf) {
        *self.0.lock() = Some(path);
    }

    pub fn take(&self) -> Option<PathBuf> {
        self.0.lock().take()
    }
}

/// The first argument after the program that names an existing folder or
/// Markdown file. Relative paths resolve against the launch directory.
pub fn requested_path(args: impl IntoIterator<Item = String>, cwd: &Path) -> Option<PathBuf> {
    args.into_iter()
        .skip(1)
        .filter(|arg| !arg.starts_with('-'))
        .map(|arg| cwd.join(arg))
        .find(|path| path.is_dir() || (path.is_file() && crate::vault::is_markdown(path)))
}

/// Maps a folder or note to the vault that contains it. A path inside a known
/// vault opens that vault, so no vault is created inside another; otherwise
/// the folder itself, or the note's folder, becomes the proposed vault.
pub fn resolve(path: &Path, vaults: &[VaultConfig]) -> Option<OpenRequest> {
    let target = std::fs::canonicalize(path).ok()?;
    let containing = vaults
        .iter()
        .filter_map(|vault| {
            let root = std::fs::canonicalize(&vault.path).ok()?;
            target.starts_with(&root).then_some((vault, root))
        })
        .max_by_key(|(_, root)| root.components().count());
    let is_dir = target.is_dir();
    if let Some((vault, root)) = containing {
        let note_path = (!is_dir).then(|| {
            target
                .strip_prefix(&root)
                .unwrap_or(&target)
                .to_string_lossy()
                .replace('\\', "/")
        });
        return Some(OpenRequest {
            vault_path: vault.path.to_string_lossy().into_owned(),
            note_path,
            known_vault: true,
        });
    }
    let folder = if is_dir { path } else { path.parent()? };
    Some(OpenRequest {
        vault_path: folder.to_string_lossy().into_owned(),
        note_path: path
            .file_name()
            .filter(|_| !is_dir)
            .map(|name| name.to_string_lossy().into_owned()),
        known_vault: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(path: &Path) -> VaultConfig {
        VaultConfig::new(path.to_path_buf(), None)
    }

    #[test]
    fn labels_follow_the_interface_language() {
        assert_eq!(label("pt-BR"), "Abrir com LowNotes");
        assert_eq!(label("es-ES"), "Abrir con LowNotes");
        assert_eq!(label(""), "Open with LowNotes");
    }

    #[test]
    fn launch_arguments_name_folders_and_markdown_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("note.md"), "# Note").unwrap();
        std::fs::write(root.path().join("image.png"), "").unwrap();
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();

        assert_eq!(
            requested_path(args(&["lownotes", "note.md"]), root.path()),
            Some(root.path().join("note.md"))
        );
        assert_eq!(
            requested_path(args(&["lownotes", "--flag", "."]), root.path()),
            Some(root.path().join("."))
        );
        assert_eq!(
            requested_path(args(&["lownotes", "image.png"]), root.path()),
            None
        );
        assert_eq!(
            requested_path(args(&["lownotes", "missing.md"]), root.path()),
            None
        );
        assert_eq!(requested_path(args(&["lownotes"]), root.path()), None);
    }

    #[test]
    fn notes_inside_a_known_vault_open_in_that_vault() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("Projects").join("Ideas");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("plan.md"), "").unwrap();
        let vaults = [vault(root.path())];

        let note = resolve(&nested.join("plan.md"), &vaults).unwrap();
        assert_eq!(note.vault_path, root.path().to_string_lossy());
        assert_eq!(note.note_path.as_deref(), Some("Projects/Ideas/plan.md"));
        assert!(note.known_vault);

        let folder = resolve(&nested, &vaults).unwrap();
        assert_eq!(folder.vault_path, root.path().to_string_lossy());
        assert_eq!(folder.note_path, None);
        assert!(folder.known_vault);
    }

    #[test]
    fn the_innermost_known_vault_wins() {
        let root = tempfile::tempdir().unwrap();
        let inner = root.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("a.md"), "").unwrap();
        let vaults = [vault(root.path()), vault(&inner)];

        let note = resolve(&inner.join("a.md"), &vaults).unwrap();
        assert_eq!(note.vault_path, inner.to_string_lossy());
        assert_eq!(note.note_path.as_deref(), Some("a.md"));
    }

    #[test]
    fn paths_outside_known_vaults_propose_their_folder() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("loose.md"), "").unwrap();
        let vaults = [vault(root.path())];

        let note = resolve(&other.path().join("loose.md"), &vaults).unwrap();
        assert_eq!(note.vault_path, other.path().to_string_lossy());
        assert_eq!(note.note_path.as_deref(), Some("loose.md"));
        assert!(!note.known_vault);

        let folder = resolve(other.path(), &vaults).unwrap();
        assert_eq!(folder.vault_path, other.path().to_string_lossy());
        assert_eq!(folder.note_path, None);
        assert!(!folder.known_vault);
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_inside_the_vault() {
        let root = tempfile::tempdir().unwrap();
        let notes = root.path().join("notes");
        let sibling = root.path().join("notes-old");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();

        let request = resolve(&sibling, &[vault(&notes)]).unwrap();
        assert!(!request.known_vault);
        assert_eq!(request.vault_path, sibling.to_string_lossy());
    }

    #[test]
    fn exec_paths_are_quoted_for_desktop_entries() {
        assert_eq!(
            exec_argument("/opt/Low Notes/lownotes"),
            "\"/opt/Low Notes/lownotes\""
        );
        assert_eq!(exec_argument("/a/$b`c\"d"), "\"/a/\\\\$b\\\\`c\\\\\"d\"");
        assert_eq!(exec_argument("/a\\b"), "\"/a\\\\\\\\b\"");
        assert_eq!(exec_argument("/100%/x"), "\"/100%%/x\"");
    }

    /// Writes to the real HKEY_CURRENT_USER, so it only runs on request:
    /// `cargo test --lib context_menu::tests::registry_round_trip -- --ignored`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn registry_round_trip() {
        use windows_registry::CURRENT_USER;
        let key = r"Software\Classes\Directory\Background\shell\LowNotes";
        apply(true, "pt-BR").unwrap();
        let entry = CURRENT_USER.open(key).unwrap();
        assert_eq!(entry.get_string("").unwrap(), "Abrir com LowNotes");
        let command = entry.open("command").unwrap().get_string("").unwrap();
        let program = std::env::current_exe().unwrap();
        assert_eq!(command, format!("\"{}\" \"%V\"", program.display()));
        apply(false, "pt-BR").unwrap();
        assert!(CURRENT_USER.open(key).is_err());
        apply(false, "pt-BR").unwrap();
    }

    #[test]
    fn the_desktop_entry_is_hidden_and_covers_folders_and_markdown() {
        let entry = desktop_entry(Path::new("/opt/LowNotes.AppImage"), "Open with LowNotes");
        assert!(entry.contains("Exec=\"/opt/LowNotes.AppImage\" %f\n"));
        assert!(entry.contains("NoDisplay=true\n"));
        assert!(entry.contains("MimeType=inode/directory;text/markdown;text/x-markdown;\n"));
    }
}
