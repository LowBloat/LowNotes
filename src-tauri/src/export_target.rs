//! Export destinations are chosen with the native save dialog *in Rust*.
//! The webview only receives an opaque, single-use token, so it can never
//! make the app write to a path of its choosing.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use rand::Rng;
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

const MAX_EXPORT_BYTES: usize = 32 * 1024 * 1024;
const TOKEN_TTL: Duration = Duration::from_secs(15 * 60);
const FORBIDDEN_NAME_CHARS: &str = "<>:\"/\\|?*";

struct Pending {
    path: PathBuf,
    format: ExportFormat,
    created: Instant,
}

fn pending() -> &'static Mutex<HashMap<String, Pending>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Pdf,
    Docx,
}

impl ExportFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pdf" => Ok(Self::Pdf),
            "docx" => Ok(Self::Docx),
            _ => Err("export.invalidFile".into()),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Docx => "Word",
        }
    }

    fn accepts(self, bytes: &[u8]) -> bool {
        match self {
            Self::Pdf => bytes.starts_with(b"%PDF-"),
            Self::Docx => bytes.starts_with(b"PK\x03\x04"),
        }
    }
}

/// Suggested file name only; the user still picks the folder and the name.
fn suggested_name(name: &str, format: ExportFormat) -> String {
    let stem: String = Path::new(&name.replace('\\', "/"))
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_control() || FORBIDDEN_NAME_CHARS.contains(c) { '-' } else { c })
        .collect();
    let stem = stem.trim();
    format!("{}.{}", if stem.is_empty() { "document" } else { stem }, format.extension())
}

fn with_extension(path: PathBuf, format: ExportFormat) -> PathBuf {
    let matches = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(format.extension()));
    if matches {
        return path;
    }
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{}", format.extension()));
    path.with_file_name(name)
}

pub(crate) fn register(path: PathBuf, format: ExportFormat) -> String {
    let token = format!("{:032x}", rand::rng().random::<u128>());
    let mut map = pending().lock();
    map.retain(|_, entry| entry.created.elapsed() < TOKEN_TTL);
    map.insert(
        token.clone(),
        Pending { path: with_extension(path, format), format, created: Instant::now() },
    );
    token
}

/// Writes `bytes` to the destination behind `token`. The token is consumed
/// whether or not the write succeeds.
pub(crate) fn write(token: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let entry = pending().lock().remove(token).ok_or("export.invalidFile")?;
    if entry.created.elapsed() >= TOKEN_TTL || bytes.len() > MAX_EXPORT_BYTES || !entry.format.accepts(bytes) {
        return Err("export.invalidFile".into());
    }
    std::fs::write(&entry.path, bytes).map_err(|e| e.to_string())?;
    Ok(entry.path)
}

/// Shows the native save dialog and returns a token for the chosen path, or
/// `None` when the user cancels.
pub async fn choose<R: Runtime>(
    app: &AppHandle<R>,
    file_name: &str,
    format: ExportFormat,
) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(suggested_name(file_name, format))
        .add_filter(format.label(), &[format.extension()])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    Ok(Some(register(path, format)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_through_a_single_use_token_with_matching_content() {
        let dir = tempfile::tempdir().unwrap();
        let token = register(dir.path().join("report"), ExportFormat::Pdf);
        assert_eq!(write("not-a-token", b"%PDF-1.7"), Err("export.invalidFile".into()));
        let written = write(&token, b"%PDF-1.7 body").unwrap();
        assert_eq!(written, dir.path().join("report.pdf"));
        assert_eq!(std::fs::read(&written).unwrap(), b"%PDF-1.7 body");
        // Consumed: a second write with the same token is refused.
        assert!(write(&token, b"%PDF-1.7 again").is_err());

        let docx = register(dir.path().join("doc.DOCX"), ExportFormat::Docx);
        assert!(write(&docx, b"%PDF-1.7 not a zip").is_err());
        assert!(!dir.path().join("doc.DOCX").exists());
    }

    #[test]
    fn suggested_names_are_plain_file_names() {
        assert_eq!(suggested_name("../../etc/passwd.md", ExportFormat::Pdf), "passwd.pdf");
        assert_eq!(suggested_name("..\\..\\win\\notes.md", ExportFormat::Pdf), "notes.pdf");
        assert_eq!(suggested_name("a*b?c.md", ExportFormat::Docx), "a-b-c.docx");
        assert_eq!(suggested_name("", ExportFormat::Pdf), "document.pdf");
        assert_eq!(ExportFormat::parse("exe"), Err("export.invalidFile".into()));
    }
}
