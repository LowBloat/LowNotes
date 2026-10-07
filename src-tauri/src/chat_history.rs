use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::{bail, Context};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::{assistant::WebSource, rag::RagChunk};

const VERSION: u8 = 1;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
static FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDraft {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub saved_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEdit {
    #[serde(flatten)]
    pub edit: crate::assistant::NoteEdit,
    #[serde(default)]
    pub applied_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEntry {
    pub role: String,
    pub content: String,
    pub timestamp: String,
    #[serde(default)]
    pub sources: Vec<RagChunk>,
    #[serde(default)]
    pub web_sources: Vec<WebSource>,
    #[serde(default)]
    pub drafts: Vec<ChatDraft>,
    #[serde(default)]
    pub edits: Vec<ChatEdit>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub vault_id: Option<String>,
    #[serde(default)]
    pub applied_links: Option<usize>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub error_settings_tab: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatConversation {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<ChatEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistory {
    pub version: u8,
    pub active_conversation_id: Option<String>,
    #[serde(default)]
    pub conversations: Vec<ChatConversation>,
    #[serde(default)]
    pub memory: String,
}

impl Default for ChatHistory {
    fn default() -> Self {
        Self {
            version: VERSION,
            active_conversation_id: None,
            conversations: Vec::new(),
            memory: String::new(),
        }
    }
}

pub fn path_for_vault(vault_id: &str) -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "lowbloat", "lownotes").context("errors.configDir")?;
    let safe_name = blake3::hash(vault_id.as_bytes()).to_hex();
    Ok(dirs
        .data_local_dir()
        .join("chat-history")
        .join(format!("{safe_name}.json")))
}

fn read_at(path: &Path) -> anyhow::Result<ChatHistory> {
    let valid = |bytes: &[u8]| bytes.len() <= MAX_FILE_BYTES && serde_json::from_slice::<ChatHistory>(bytes)
        .is_ok_and(|history| validate(&history).is_ok());
    // Migrate the legacy .bak filename before asking the shared recovery layer to restore it.
    if !fs::read(path).is_ok_and(|bytes| valid(&bytes)) {
        let legacy = path.with_extension("bak");
        let backup = crate::storage::backup_path(path);
        if let Ok(bytes) = fs::read(&legacy) {
            let newer = fs::metadata(&legacy).and_then(|meta| meta.modified()).ok()
                > fs::metadata(&backup).and_then(|meta| meta.modified()).ok();
            if valid(&bytes) && (newer || !backup.exists()) {
                crate::storage::write_validated(&backup, &bytes, valid)?;
            }
        }
    }
    let bytes = crate::storage::read_validated(path, valid).map_err(|_| anyhow::anyhow!("ai.historyCorrupt"))?;
    match bytes { Some(bytes) => Ok(serde_json::from_slice(&bytes)?), None => Ok(ChatHistory::default()) }
}

pub fn load(vault_id: &str) -> anyhow::Result<ChatHistory> {
    let _guard = FILE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("chat history lock");
    read_at(&path_for_vault(vault_id)?)
}

fn validate(history: &ChatHistory) -> anyhow::Result<Vec<u8>> {
    if history.version != VERSION
        || history.memory.chars().count() > 8_000
        || history.conversations.len() > 1000
    {
        bail!("ai.historyTooLarge");
    }
    for chat in &history.conversations {
        if chat.id.is_empty()
            || chat.id.len() > 100
            || chat.title.len() > 200
            || chat.messages.len() > 5000
        {
            bail!("ai.historyTooLarge");
        }
        for entry in &chat.messages {
            if !matches!(entry.role.as_str(), "user" | "assistant")
                || entry.content.len() > 1024 * 1024
                || entry.drafts.len() > 20
                || entry.edits.len() > 20
                || entry.edits.iter().any(|edit| edit.edit.old_text.len() > 256 * 1024 || edit.edit.new_text.len() > 256 * 1024)
                || entry
                    .drafts
                    .iter()
                    .any(|draft| draft.content.len() > 256 * 1024)
            {
                bail!("ai.historyTooLarge");
            }
        }
    }
    let bytes = serde_json::to_vec(history)?;
    if bytes.len() > MAX_FILE_BYTES {
        bail!("ai.historyTooLarge");
    }
    Ok(bytes)
}

fn save_at(path: &Path, history: &ChatHistory) -> anyhow::Result<()> {
    let bytes = validate(history)?;
    crate::storage::write_validated(path, &bytes, |data| {
        serde_json::from_slice::<ChatHistory>(data).is_ok_and(|history| validate(&history).is_ok())
    })
}

pub fn save(vault_id: &str, history: &ChatHistory) -> anyhow::Result<()> {
    let _guard = FILE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("chat history lock");
    save_at(&path_for_vault(vault_id)?, history)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_conversations_memory_and_recovers_backup() {
        let dir =
            std::env::temp_dir().join(format!("lownotes-chat-history-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("chat.json");
        let mut history = ChatHistory::default();
        history.memory = "Prefiro respostas em português.".into();
        history.active_conversation_id = Some("chat-1".into());
        history.conversations.push(ChatConversation {
            id: "chat-1".into(),
            title: "Aprender Python".into(),
            created_at: 1,
            updated_at: 2,
            messages: vec![ChatEntry {
                role: "user".into(),
                content: "Olá".into(),
                timestamp: "09:00".into(),
                sources: vec![],
                web_sources: vec![],
                drafts: vec![ChatDraft {
                    path: "Python/Plano.md".into(),
                    content: "# Plano".into(),
                    saved_path: Some("Python/Plano.md".into()),
                }],
                warnings: vec![],
                edits: vec![ChatEdit {
                    edit: crate::assistant::NoteEdit { path: "Python/Plano.md".into(), old_text: "- [ ] Estudar".into(), new_text: "- [x] Estudar".into() },
                    applied_path: Some("Python/Plano.md".into()),
                }],
                vault_id: None,
                applied_links: None,
                is_error: false,
                error_settings_tab: None,
            }],
        });
        save_at(&path, &history).unwrap();
        let value = serde_json::to_value(&history).unwrap();
        assert_eq!(value["activeConversationId"], "chat-1");
        assert_eq!(value["conversations"][0]["messages"][0]["edits"][0]["old_text"], "- [ ] Estudar");
        assert_eq!(read_at(&path).unwrap().conversations[0].messages[0].edits[0].applied_path.as_deref(), Some("Python/Plano.md"));
        assert_eq!(
            read_at(&path).unwrap().conversations[0].messages[0].content,
            "Olá"
        );
        assert_eq!(
            read_at(&path).unwrap().conversations[0].messages[0].drafts[0]
                .saved_path
                .as_deref(),
            Some("Python/Plano.md")
        );
        save_at(&path, &history).unwrap();
        assert!(!path.with_extension("bak").exists());
        fs::copy(&path, path.with_extension("bak")).unwrap();
        fs::write(&path, "invalid").unwrap();
        assert_eq!(read_at(&path).unwrap().memory, history.memory);
        let _ = fs::remove_dir_all(&dir);
    }
}
