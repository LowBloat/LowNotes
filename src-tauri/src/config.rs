use std::{
    fs,
    path::PathBuf,
};

use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use directories::ProjectDirs;
use iroh::{EndpointAddr, EndpointId, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use rand::Rng;
use serde::{Deserialize, Serialize};

const PAIR_CODE_PREFIX: &str = "LOWNOTES2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerConfig {
    pub name: String,
    pub endpoint_id: String,
    pub ticket: String,
}

impl PeerConfig {
    pub fn endpoint_addr(&self) -> anyhow::Result<EndpointAddr> {
        let ticket: EndpointTicket = self
            .ticket
            .parse()
            .context("errors.invalidDeviceAddress")?;
        Ok(ticket.endpoint_addr().clone())
    }

    pub fn id(&self) -> anyhow::Result<EndpointId> {
        self.endpoint_id
            .parse()
            .context("errors.invalidDeviceId")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairInvite {
    pub peer: PeerConfig,
    pub token: String,
    pub vault_id: String,
    pub vault_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub secret_key: String,
    #[serde(default)]
    pub pairing_token: String,
    #[serde(default, skip_deserializing)]
    pub credentials_locked: bool,
    #[serde(default)]
    pub peers: Vec<PeerConfig>,
}

impl VaultConfig {
    pub fn new(path: PathBuf, name: Option<String>) -> Self {
        let vault_name = name.unwrap_or_else(|| {
            path.file_name()
                .and_then(|v| v.to_str())
                .filter(|v| !v.trim().is_empty())
                .unwrap_or("Vault")
                .to_owned()
        });

        Self {
            id: URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 18]>()),
            name: vault_name,
            path,
            secret_key: URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>()),
            pairing_token: new_pairing_token(),
            peers: Vec::new(),
            credentials_locked: false,
        }
    }

    pub fn secret_key(&self) -> anyhow::Result<SecretKey> {
        decode_secret(&self.secret_key)
    }

    pub fn ensure_keys(&mut self) -> bool {
        if self.credentials_locked { return false; }
        let mut changed = false;
        if self.pairing_token.trim().is_empty() {
            self.pairing_token = new_pairing_token();
            changed = true;
        }
        if decode_secret(&self.secret_key).is_err() {
            self.secret_key = URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>());
            changed = true;
        }
        changed
    }

    pub fn add_peer(&mut self, peer: PeerConfig) -> bool {
        if self.peers.iter().any(|p| p.endpoint_id == peer.endpoint_id) {
            false
        } else {
            self.peers.push(peer);
            true
        }
    }

    pub fn remove_peer(&mut self, endpoint_id: &str) -> bool {
        let prev_len = self.peers.len();
        self.peers.retain(|p| p.endpoint_id != endpoint_id);
        self.peers.len() < prev_len
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub selected_model: String,
    #[serde(default)]
    pub is_custom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSettings {
    pub active_provider_id: String,
    pub providers: Vec<AiProviderConfig>,
    #[serde(default)]
    pub auto_link_notes: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            active_provider_id: "ollama".to_string(),
            providers: vec![
                AiProviderConfig {
                    id: "ollama".to_string(),
                    name: "Ollama (Local)".to_string(),
                    base_url: "http://localhost:11434/v1".to_string(),
                    api_key: String::new(),
                    selected_model: "qwen2.5:1.5b".to_string(),
                    is_custom: false,
                },
                AiProviderConfig {
                    id: "openai".to_string(),
                    name: "OpenAI".to_string(),
                    base_url: "https://api.openai.com/v1".to_string(),
                    api_key: String::new(),
                    selected_model: "gpt-4o-mini".to_string(),
                    is_custom: false,
                },
                AiProviderConfig {
                    id: "openrouter".to_string(),
                    name: "OpenRouter".to_string(),
                    base_url: "https://openrouter.ai/api/v1".to_string(),
                    api_key: String::new(),
                    selected_model: "meta-llama/llama-3.3-70b-instruct:free".to_string(),
                    is_custom: false,
                },
                AiProviderConfig {
                    id: "groq".to_string(),
                    name: "Groq".to_string(),
                    base_url: "https://api.groq.com/openai/v1".to_string(),
                    api_key: String::new(),
                    selected_model: "llama-3.3-70b-versatile".to_string(),
                    is_custom: false,
                },
                AiProviderConfig {
                    id: "lmstudio".to_string(),
                    name: "LM Studio (Local)".to_string(),
                    base_url: "http://localhost:1234/v1".to_string(),
                    api_key: String::new(),
                    selected_model: "local-model".to_string(),
                    is_custom: false,
                },
            ],
            auto_link_notes: false,
        }
    }
}

impl AiSettings {
    /// Re-applies built-in providers on load so new defaults ship in any
    /// version; user edits to built-ins and custom providers are preserved.
    pub fn normalize(&mut self) {
        let saved = std::mem::take(&mut self.providers);
        let mut providers: Vec<AiProviderConfig> = Self::default()
            .providers
            .into_iter()
            .map(|default| saved.iter().find(|p| p.id == default.id).cloned().unwrap_or(default))
            .collect();
        providers.extend(saved.into_iter().filter(|p| p.is_custom));
        self.providers = providers;
        if !self.providers.iter().any(|p| p.id == self.active_provider_id) {
            self.active_provider_id = self.providers.first().map(|p| p.id.clone()).unwrap_or_default();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchSourceConfig {
    pub id: String,
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebSearchSettings {
    pub sources: Vec<WebSearchSourceConfig>,
    pub searxng_url: String,
}

impl Default for WebSearchSettings {
    fn default() -> Self {
        Self {
            sources: ["firecrawl", "keenable", "exa", "duckduckgo", "searxng", "brave", "parallel"]
                .into_iter()
                .map(|id| WebSearchSourceConfig {
                    id: id.into(),
                    enabled: !matches!(id, "brave" | "parallel"),
                    api_key: String::new(),
                })
                .collect(),
            searxng_url: "https://search.lumy.live/".into(),
        }
    }
}

impl WebSearchSettings {
    pub fn source(&self, id: &str) -> Option<&WebSearchSourceConfig> {
        self.sources.iter().find(|source| source.id == id)
    }

    pub fn normalize(&mut self) {
        let defaults = Self::default();
        self.sources = defaults.sources.into_iter().map(|default| {
            self.sources.iter().find(|source| source.id == default.id).cloned().unwrap_or(default)
        }).collect();
    }
}

/// Palettes shipped with the app. They live in code (and in the frontend
/// `themes.ts` mirror) so new defaults can be added in any version without
/// touching user data; only `custom_palettes` below is persisted.
pub const BUILTIN_PALETTE_IDS: [&str; 3] = ["lowbloat", "megumin", "rimuru"];

/// One color per UI token, stored as `#rrggbb`. Derived tokens
/// (glow/selection/highlight) are computed by the frontend.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ThemeColors {
    pub bg_main: String,
    pub bg_sidebar: String,
    pub bg_card: String,
    pub bg_hover: String,
    pub bg_active: String,
    pub border: String,
    pub text_main: String,
    pub text_muted: String,
    pub text_dim: String,
    pub accent: String,
    pub accent_light: String,
    pub accent_contrast: String,
    pub success: String,
    pub danger: String,
}

impl ThemeColors {
    pub fn values(&self) -> impl Iterator<Item = &str> {
        [
            &self.bg_main, &self.bg_sidebar, &self.bg_card, &self.bg_hover, &self.bg_active,
            &self.border, &self.text_main, &self.text_muted, &self.text_dim, &self.accent,
            &self.accent_light, &self.accent_contrast, &self.success, &self.danger,
        ]
        .into_iter()
        .map(|value| value.as_str())
    }

    pub fn is_valid(&self) -> bool {
        self.values().all(is_hex_color)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemePalette {
    pub id: String,
    pub name: String,
    pub dark: ThemeColors,
    pub light: ThemeColors,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThemePalettesSettings {
    #[serde(default = "default_palette_id")]
    pub active_palette_id: String,
    #[serde(default)]
    pub custom_palettes: Vec<ThemePalette>,
}

impl Default for ThemePalettesSettings {
    fn default() -> Self {
        Self {
            active_palette_id: default_palette_id(),
            custom_palettes: Vec::new(),
        }
    }
}

impl ThemePalettesSettings {
    /// Mirrors the `save_theme_palettes` command validation so loading a
    /// hand-edited/corrupted file degrades to the default palette.
    pub fn normalize(&mut self) {
        self.custom_palettes.retain(|palette| {
            let id = palette.id.trim();
            !id.is_empty()
                && !BUILTIN_PALETTE_IDS.contains(&id)
                && !palette.name.trim().is_empty()
                && palette.dark.values().chain(palette.light.values()).all(is_hex_color)
        });
        let mut seen = std::collections::HashSet::new();
        self.custom_palettes.retain(|palette| seen.insert(palette.id.clone()));
        let known = self.custom_palettes.iter().any(|palette| palette.id == self.active_palette_id)
            || BUILTIN_PALETTE_IDS.contains(&self.active_palette_id.as_str());
        if !known {
            self.active_palette_id = default_palette_id();
        }
    }
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn default_palette_id() -> String {
    "lowbloat".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageUploadSettings {
    pub provider: String,
    #[serde(default)]
    pub local_default_applied: bool,
    /// Public application identifier, never a Client Secret. Empty uses LowNotes' ID.
    pub imgur_client_id: String,
}

impl Default for ImageUploadSettings {
    fn default() -> Self {
        Self { provider: "local".into(), imgur_client_id: String::new(), local_default_applied: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub credential_storage_version: u8,
    #[serde(default)]
    pub credential_refs: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_deserializing)]
    pub credential_error: String,
    #[serde(skip)]
    pub unavailable_credentials: std::collections::BTreeSet<String>,
    pub device_name: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub theme_palettes: ThemePalettesSettings,
    #[serde(default = "default_view_mode")]
    pub view_mode: String,
    #[serde(default = "default_true")]
    pub line_wrapping: bool,
    #[serde(default)]
    pub language: String,
    pub active_vault_id: Option<String>,
    pub vaults: Vec<VaultConfig>,
    #[serde(default)]
    pub ai: AiSettings,
    #[serde(default)]
    pub web_search: WebSearchSettings,
    #[serde(default)]
    pub image_upload: ImageUploadSettings,
    #[serde(default)]
    pub has_seen_welcome: bool,
    #[serde(default = "default_true")]
    pub update_check: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default)]
    pub skipped_version: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let host = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "LowNotes Device".to_string());

        Self {
            device_name: host,
            credential_storage_version: 0,
            credential_refs: Default::default(),
            credential_error: String::new(),
            unavailable_credentials: Default::default(),
            theme: default_theme(),
            view_mode: default_view_mode(),
            line_wrapping: default_true(),
            theme_palettes: ThemePalettesSettings::default(),
            language: String::new(),
            active_vault_id: None,
            vaults: Vec::new(),
            ai: AiSettings::default(),
            web_search: WebSearchSettings::default(),
            image_upload: ImageUploadSettings::default(),
            has_seen_welcome: false,
            update_check: default_true(),
            close_to_tray: default_true(),
            skipped_version: String::new(),
        }
    }
}

fn default_theme() -> String {
    "light".to_string()
}

fn default_view_mode() -> String {
    "split".to_string()
}

fn default_true() -> bool {
    true
}

impl AppSettings {
    pub(crate) fn from_saved_value(saved: serde_json::Value) -> Option<Self> {
        let old_brave_key = saved.pointer("/ai/web_search_api_key")
            .and_then(|key| key.as_str()).unwrap_or_default().to_owned();
        let mut settings: Self = serde_json::from_value(saved).ok()?;
        if settings.credential_storage_version > 1 { return None; }
        if settings.credential_storage_version == 1 && settings.vaults.iter().any(|vault|
            !settings.credential_refs.contains_key(&format!("vault:{}:identity", vault.id))
            || !settings.credential_refs.contains_key(&format!("vault:{}:pairing", vault.id))) { return None; }
        settings.web_search.normalize();
        settings.theme_palettes.normalize();
        settings.ai.normalize();
        if !settings.image_upload.local_default_applied || !matches!(settings.image_upload.provider.as_str(), "local" | "catbox" | "imgur") {
            settings.image_upload.provider = "local".into();
            settings.image_upload.local_default_applied = true;
        }
        if !old_brave_key.is_empty() {
            if let Some(brave) = settings.web_search.sources.iter_mut().find(|s| s.id == "brave") {
                if brave.api_key.is_empty() {
                    brave.api_key = old_brave_key;
                    brave.enabled = true;
                }
            }
        }
        Some(settings)
    }

    pub fn config_file() -> anyhow::Result<PathBuf> {
        let dirs = ProjectDirs::from("dev", "lowbloat", "lownotes")
            .context("errors.configDir")?;
        let dir = dirs.config_dir();
        fs::create_dir_all(dir)?;
        Ok(dir.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::config_file()
            .ok()
            .and_then(|p| crate::credentials::load(&p).ok())
            .unwrap_or_default()
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        let path = Self::config_file()?;
        crate::credentials::save(&path, self)?;
        Ok(())
    }

    pub(crate) fn valid_saved_bytes(bytes: &[u8]) -> bool {
        serde_json::from_slice::<serde_json::Value>(bytes).ok().and_then(Self::from_saved_value).is_some()
    }

    pub fn active_vault(&self) -> Option<&VaultConfig> {
        let id = self.active_vault_id.as_deref()?;
        self.vaults.iter().find(|v| v.id == id)
    }

    pub fn active_vault_mut(&mut self) -> Option<&mut VaultConfig> {
        let id = self.active_vault_id.clone()?;
        self.vaults.iter_mut().find(|v| v.id == id)
    }
}

pub fn new_pairing_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 16]>())
}

pub fn decode_secret(encoded: &str) -> anyhow::Result<SecretKey> {
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("errors.invalidSecretKey")?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("errors.invalidKeySize"))?;
    Ok(SecretKey::from(key))
}

pub fn encode_pair_code(invite: &PairInvite) -> anyhow::Result<String> {
    let json = serde_json::to_vec(invite)?;
    let encoded = URL_SAFE_NO_PAD.encode(json);
    Ok(format!("{PAIR_CODE_PREFIX}_{encoded}"))
}

pub fn decode_pair_code(code: &str) -> anyhow::Result<PairInvite> {
    let trimmed = code.trim();
    let body = trimmed
        .strip_prefix(&format!("{PAIR_CODE_PREFIX}_"))
        .or_else(|| trimmed.strip_prefix(PAIR_CODE_PREFIX))
        .context("errors.invalidPairCode")?;
    let json = URL_SAFE_NO_PAD.decode(body)?;
    let invite = serde_json::from_slice(&json)?;
    Ok(invite)
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, ThemeColors, ThemePalette, ThemePalettesSettings};

    #[test]
    fn damaged_settings_recover_the_same_vault_identity_and_preserve_the_original_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let mut settings = AppSettings::default();
        let vault = super::VaultConfig::new(root.path().to_path_buf(), None);
        let identity = vault.secret_key.clone();
        let token = vault.pairing_token.clone();
        settings.active_vault_id = Some(vault.id.clone());
        settings.vaults.push(vault);
        let original = serde_json::to_vec(&settings).unwrap();
        crate::storage::write_validated(&path, &original, AppSettings::valid_saved_bytes).unwrap();
        settings.theme = "dark".into();
        crate::storage::write_validated(&path, &serde_json::to_vec(&settings).unwrap(), AppSettings::valid_saved_bytes).unwrap();
        std::fs::write(&path, b"interrupted settings").unwrap();
        let bytes = crate::storage::read_validated(&path, AppSettings::valid_saved_bytes).unwrap().unwrap();
        let recovered = AppSettings::from_saved_value(serde_json::from_slice(&bytes).unwrap()).unwrap();
        assert_eq!(recovered.vaults[0].secret_key, identity);
        assert_eq!(recovered.vaults[0].pairing_token, token);
        assert!(std::fs::read_dir(root.path()).unwrap().any(|entry| std::fs::read(entry.unwrap().path()).ok().as_deref() == Some(b"interrupted settings")));
    }

    #[test]
    fn incomplete_settings_are_rejected_instead_of_becoming_a_new_identity() {
        assert!(!AppSettings::valid_saved_bytes(br#"{}"#));
        assert!(!AppSettings::valid_saved_bytes(br#"{"vaults":[]}"#));
    }

    #[test]
    fn image_host_settings_migrate_and_round_trip_without_resetting_user_data() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved["device_name"] = "Existing device".into();
        saved.as_object_mut().unwrap().remove("image_upload");
        let mut restored = AppSettings::from_saved_value(saved).unwrap();
        assert_eq!(restored.image_upload.provider, "local");
        assert_eq!(restored.device_name, "Existing device");
        restored.image_upload.provider = "imgur".into();
        restored.image_upload.imgur_client_id = "custom123".into();
        let mut saved = serde_json::to_value(restored).unwrap();
        let restored = AppSettings::from_saved_value(saved.clone()).unwrap();
        assert_eq!(restored.image_upload.provider, "imgur");
        assert_eq!(restored.image_upload.imgur_client_id, "custom123");
        saved["image_upload"]["provider"] = "unknown-future-host".into();
        let restored = AppSettings::from_saved_value(saved).unwrap();
        assert_eq!(restored.device_name, "Existing device");
        assert_eq!(restored.image_upload.provider, "local");
        assert_eq!(restored.image_upload.imgur_client_id, "custom123");
    }

    #[test]
    fn existing_host_settings_switch_to_local_once_and_keep_later_choices() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved["image_upload"]["provider"] = "imgur".into();
        saved["image_upload"]["imgur_client_id"] = "myId123".into();
        saved["image_upload"].as_object_mut().unwrap().remove("local_default_applied");
        let mut restored = AppSettings::from_saved_value(saved).unwrap();
        assert_eq!(restored.image_upload.provider, "local");
        assert_eq!(restored.image_upload.imgur_client_id, "myId123");
        restored.image_upload.provider = "catbox".into();
        let restored = AppSettings::from_saved_value(serde_json::to_value(restored).unwrap()).unwrap();
        assert_eq!(restored.image_upload.provider, "catbox");
    }

    #[test]
    fn existing_settings_without_view_mode_open_split() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("view_mode");

        let restored: AppSettings = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.view_mode, "split");
        assert_eq!(restored.theme, "light");
    }

    #[test]
    fn existing_settings_default_to_tray_on_close() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("close_to_tray");
        let restored: AppSettings = serde_json::from_value(saved).unwrap();
        assert!(restored.close_to_tray);
    }

    #[test]
    fn line_wrapping_defaults_on_and_preserves_saved_preference() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("line_wrapping");
        let restored: AppSettings = serde_json::from_value(saved).unwrap();
        assert!(restored.line_wrapping);

        let mut disabled = restored;
        disabled.line_wrapping = false;
        let serialized = serde_json::to_string(&disabled).unwrap();
        let reloaded: AppSettings = serde_json::from_str(&serialized).unwrap();
        assert!(!reloaded.line_wrapping);
    }

    #[test]
    fn test_vault_config_ensure_keys() {
        let mut vault = super::VaultConfig {
            id: "v1".to_string(),
            name: "Test".to_string(),
            path: std::path::PathBuf::from("/test"),
            secret_key: "".to_string(),
            pairing_token: "".to_string(),
            credentials_locked: false,
            peers: Vec::new(),
        };
        assert!(vault.ensure_keys());
        assert!(!vault.secret_key.is_empty());
        assert!(!vault.pairing_token.is_empty());
        assert!(vault.secret_key().is_ok());
        // Second time should not change anything
        assert!(!vault.ensure_keys());
    }

    #[test]
    fn migrates_old_brave_key_without_making_brave_default() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("web_search");
        saved["ai"]["web_search_api_key"] = "old-key".into();
        let restored = AppSettings::from_saved_value(saved).unwrap();
        let brave = restored.web_search.source("brave").unwrap();
        assert!(brave.enabled);
        assert_eq!(brave.api_key, "old-key");
        assert!(restored.web_search.source("firecrawl").unwrap().enabled);
    }

    fn colors(hex: &str) -> ThemeColors {
        ThemeColors {
            bg_main: hex.into(), bg_sidebar: hex.into(), bg_card: hex.into(),
            bg_hover: hex.into(), bg_active: hex.into(), border: hex.into(),
            text_main: hex.into(), text_muted: hex.into(), text_dim: hex.into(),
            accent: hex.into(), accent_light: hex.into(), accent_contrast: hex.into(),
            success: hex.into(), danger: hex.into(),
        }
    }

    #[test]
    fn theme_palettes_default_to_lowbloat() {
        let settings = ThemePalettesSettings::default();
        assert_eq!(settings.active_palette_id, "lowbloat");
        assert!(settings.custom_palettes.is_empty());
    }

    #[test]
    fn normalize_keeps_valid_custom_palette_and_active_choice() {
        let mut settings = ThemePalettesSettings {
            active_palette_id: "custom_x".into(),
            custom_palettes: vec![ThemePalette {
                id: "custom_x".into(), name: "Ocean".into(),
                dark: colors("#101010"), light: colors("#f0f0f0"),
            }],
        };
        settings.normalize();
        assert_eq!(settings.active_palette_id, "custom_x");
        assert_eq!(settings.custom_palettes.len(), 1);
    }

    #[test]
    fn normalize_drops_invalid_and_builtin_colliding_palettes() {
        let mut settings = ThemePalettesSettings {
            active_palette_id: "megumin".into(),
            custom_palettes: vec![
                ThemePalette { id: "rimuru".into(), name: "Clash".into(), dark: colors("#101010"), light: colors("#f0f0f0") },
                ThemePalette { id: "custom_bad".into(), name: "Bad".into(), dark: colors("nope"), light: colors("#f0f0f0") },
                ThemePalette { id: "".into(), name: "Empty".into(), dark: colors("#101010"), light: colors("#f0f0f0") },
            ],
        };
        settings.normalize();
        assert!(settings.custom_palettes.is_empty());
    }

    #[test]
    fn normalize_falls_back_to_lowbloat_for_unknown_active() {
        let mut settings = ThemePalettesSettings {
            active_palette_id: "custom_gone".into(),
            custom_palettes: vec![],
        };
        settings.normalize();
        assert_eq!(settings.active_palette_id, "lowbloat");
    }

    #[test]
    fn existing_settings_without_theme_palettes_default_to_lowbloat() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("theme_palettes");
        let restored = AppSettings::from_saved_value(saved).unwrap();
        assert_eq!(restored.theme_palettes.active_palette_id, "lowbloat");
        assert!(restored.theme_palettes.custom_palettes.is_empty());
    }

    #[test]
    fn saved_legacy_palette_and_mode_remain_selected() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved["theme"] = "dark".into();
        saved["theme_palettes"]["active_palette_id"] = "megumin".into();
        let restored = AppSettings::from_saved_value(saved).unwrap();
        assert_eq!(restored.theme, "dark");
        assert_eq!(restored.theme_palettes.active_palette_id, "megumin");
    }

    #[test]
    fn normalize_restores_missing_builtin_providers_and_keeps_user_data() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved["ai"]["providers"] = serde_json::json!([
            { "id": "openai", "name": "OpenAI", "base_url": "https://api.openai.com/v1", "api_key": "sk-user", "selected_model": "gpt-4o", "is_custom": false },
            { "id": "custom_z", "name": "Mine", "base_url": "http://localhost:9999/v1", "api_key": "", "selected_model": "", "is_custom": true },
        ]);
        saved["ai"]["active_provider_id"] = "custom_z".into();
        let restored = AppSettings::from_saved_value(saved).unwrap();
        let ids: Vec<&str> = restored.ai.providers.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"ollama"), "removed defaults must come back: {ids:?}");
        assert!(ids.contains(&"custom_z"));
        let openai = restored.ai.providers.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(openai.api_key, "sk-user");
        assert_eq!(openai.selected_model, "gpt-4o");
        assert_eq!(restored.ai.active_provider_id, "custom_z");
    }

    #[test]
    fn normalize_fixes_unknown_active_provider() {
        let mut saved = serde_json::to_value(AppSettings::default()).unwrap();
        saved["ai"]["active_provider_id"] = "gone".into();
        let restored = AppSettings::from_saved_value(saved).unwrap();
        assert_eq!(restored.ai.active_provider_id, restored.ai.providers[0].id);
    }
}
