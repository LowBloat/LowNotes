//! Immutable credential references make settings/backup replacement transactional.
use crate::{config::AppSettings, storage};
use anyhow::{bail, Context};
use parking_lot::Mutex;
use rand::Rng;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::OnceLock,
};

const SERVICE: &str = "dev.lowbloat.lownotes";
const UNAVAILABLE: &str = "credentials.unavailable";

trait Store {
    fn get(&mut self, reference: &str) -> anyhow::Result<String>;
    fn put(&mut self, reference: &str, secret: &str) -> anyhow::Result<()>;
}

#[derive(Default)]
struct NativeStore {
    cache: BTreeMap<String, String>,
}
impl Store for NativeStore {
    fn get(&mut self, reference: &str) -> anyhow::Result<String> {
        if let Some(secret) = self.cache.get(reference) {
            return Ok(secret.clone());
        }
        let secret = keyring::Entry::new(SERVICE, reference)?.get_password()?;
        self.cache.insert(reference.into(), secret.clone());
        Ok(secret)
    }
    fn put(&mut self, reference: &str, secret: &str) -> anyhow::Result<()> {
        let entry = keyring::Entry::new(SERVICE, reference)?;
        entry.set_password(secret)?;
        if entry.get_password()? != secret {
            bail!(UNAVAILABLE);
        }
        self.cache.insert(reference.into(), secret.into());
        Ok(())
    }
}
static NATIVE: OnceLock<Mutex<NativeStore>> = OnceLock::new();

fn visit(settings: &mut AppSettings, mut f: impl FnMut(String, &mut String)) {
    for vault in &mut settings.vaults {
        f(
            format!("vault:{}:identity", vault.id),
            &mut vault.secret_key,
        );
        f(
            format!("vault:{}:pairing", vault.id),
            &mut vault.pairing_token,
        );
    }
    for provider in &mut settings.ai.providers {
        f(format!("ai:{}", provider.id), &mut provider.api_key);
    }
    for source in &mut settings.web_search.sources {
        f(format!("web:{}", source.id), &mut source.api_key);
    }
}

fn hydrate(settings: &mut AppSettings, store: &mut impl Store) {
    let references = settings.credential_refs.clone();
    let mut unavailable = BTreeSet::new();
    visit(settings, |slot, value| {
        if let Some(reference) = references.get(&slot) {
            match store.get(reference) {
                Ok(secret) => *value = secret,
                Err(_) => {
                    value.clear();
                    unavailable.insert(slot);
                }
            }
        }
    });
    for vault in &mut settings.vaults {
        vault.credentials_locked = unavailable.contains(&format!("vault:{}:identity", vault.id))
            || unavailable.contains(&format!("vault:{}:pairing", vault.id));
    }
    settings.credential_error = if unavailable.is_empty() {
        String::new()
    } else {
        UNAVAILABLE.into()
    };
    settings.unavailable_credentials = unavailable;
}

/// For exporting settings: no keys, pairing tokens, or references into the OS store.
pub fn export_value(settings: &AppSettings) -> anyhow::Result<serde_json::Value> {
    let mut public = settings.clone();
    visit(&mut public, |_, value| value.clear());
    public.credential_refs.clear();
    public.credential_storage_version = 0;
    public.credential_error.clear();
    let mut value = serde_json::to_value(public)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("credential_refs");
        object.remove("credential_storage_version");
        object.remove("credential_error");
    }
    if let Some(vaults) = value["vaults"].as_array_mut() {
        for vault in vaults {
            vault
                .as_object_mut()
                .context("invalid vault configuration")?
                .remove("credentials_locked");
        }
    }
    Ok(value)
}

fn save_at(
    path: &Path,
    settings: &AppSettings,
    store: &mut impl Store,
) -> anyhow::Result<AppSettings> {
    // Never replace an unrecoverable settings file with defaults.
    storage::read_validated(path, AppSettings::valid_saved_bytes)?;
    let mut next = settings.clone();
    let mut references = BTreeMap::new();
    let mut error = None;
    visit(&mut next, |slot, value| {
        if error.is_some() {
            return;
        }
        if value.is_empty() {
            if settings.unavailable_credentials.contains(&slot) {
                if let Some(reference) = settings.credential_refs.get(&slot) {
                    references.insert(slot, reference.clone());
                }
            }
            return;
        }
        if let Some(reference) = settings.credential_refs.get(&slot) {
            if store.get(reference).is_ok_and(|saved| saved == *value) {
                references.insert(slot, reference.clone());
                return;
            }
        }
        let reference = format!("v1-{:032x}", rand::rng().random::<u128>());
        // Verify again through the store abstraction before removing the plaintext.
        if store.put(&reference, value).is_err()
            || !store.get(&reference).is_ok_and(|saved| saved == *value)
        {
            error = Some(anyhow::anyhow!(UNAVAILABLE));
            return;
        }
        references.insert(slot, reference);
    });
    if let Some(error) = error {
        return Err(error);
    }
    next.credential_refs = references;
    next.credential_storage_version = 1;
    let mut public = export_value(&next)?;
    public["credential_refs"] = serde_json::to_value(&next.credential_refs)?;
    public["credential_storage_version"] = 1.into();
    let bytes = serde_json::to_vec_pretty(&public)?;
    storage::write_validated(path, &bytes, AppSettings::valid_saved_bytes)?;
    // Atomic write kept the old settings as a backup. Scrub legacy plaintext only
    // after all referenced secrets are verified and the new primary is committed.
    let backup = storage::backup_path(path);
    if let Ok(previous) = fs::read(&backup) {
        if let Some(old) = serde_json::from_slice(&previous)
            .ok()
            .and_then(AppSettings::from_saved_value)
        {
            if old.credential_storage_version == 0 && next.unavailable_credentials.is_empty() {
                if storage::write_without_backup(&backup, &bytes, AppSettings::valid_saved_bytes)
                    .is_err()
                {
                    next.credential_error = "credentials.migrationIncomplete".into();
                }
            }
        }
    }
    Ok(next)
}

fn load_at(path: &Path, store: &mut impl Store) -> anyhow::Result<AppSettings> {
    let Some(bytes) = storage::read_validated(path, AppSettings::valid_saved_bytes)? else {
        return Ok(AppSettings::default());
    };
    let mut settings = AppSettings::from_saved_value(serde_json::from_slice(&bytes)?)
        .context("storage.corrupt")?;
    hydrate(&mut settings, store);
    if settings.credential_storage_version == 0 {
        match save_at(path, &settings, store) {
            Ok(migrated) => settings = migrated,
            Err(_) => settings.credential_error = UNAVAILABLE.into(),
        }
    } else {
        // Resume backup scrubbing after a crash in the migration cleanup phase.
        let backup = storage::backup_path(path);
        if settings.unavailable_credentials.is_empty()
            && fs::read(&backup)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<AppSettings>(&bytes).ok())
                .is_some_and(|old| old.credential_storage_version == 0)
        {
            if storage::write_without_backup(&backup, &bytes, AppSettings::valid_saved_bytes)
                .is_err()
            {
                settings.credential_error = "credentials.migrationIncomplete".into();
            }
        }
    }
    Ok(settings)
}

pub fn load(path: &Path) -> anyhow::Result<AppSettings> {
    load_at(path, &mut *NATIVE.get_or_init(Mutex::default).lock())
}
pub fn save(path: &Path, settings: &mut AppSettings) -> anyhow::Result<()> {
    *settings = save_at(
        path,
        settings,
        &mut *NATIVE.get_or_init(Mutex::default).lock(),
    )?;
    Ok(())
}
pub fn retry(settings: &mut AppSettings) -> anyhow::Result<()> {
    let mut store = NATIVE.get_or_init(Mutex::default).lock();
    hydrate(settings, &mut *store);
    if !settings.unavailable_credentials.is_empty() {
        bail!(UNAVAILABLE);
    }
    *settings = save_at(&AppSettings::config_file()?, settings, &mut *store)?;
    if !settings.credential_error.is_empty() {
        bail!(settings.credential_error.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VaultConfig;
    #[derive(Default)]
    struct FakeStore {
        values: BTreeMap<String, String>,
        locked: bool,
        corrupt_write: bool,
    }
    impl Store for FakeStore {
        fn get(&mut self, reference: &str) -> anyhow::Result<String> {
            if self.locked {
                bail!("locked");
            }
            self.values.get(reference).cloned().context("missing")
        }
        fn put(&mut self, reference: &str, secret: &str) -> anyhow::Result<()> {
            if self.locked {
                bail!("locked");
            }
            self.values.insert(
                reference.into(),
                if self.corrupt_write {
                    "invalid".into()
                } else {
                    secret.into()
                },
            );
            Ok(())
        }
    }
    fn legacy(path: &Path) -> AppSettings {
        let mut settings = AppSettings::default();
        let vault = VaultConfig::new(path.parent().unwrap().into(), None);
        settings.active_vault_id = Some(vault.id.clone());
        settings.vaults.push(vault);
        settings.ai.providers[0].api_key = "test-provider-secret".into();
        settings.web_search.sources[0].api_key = "test-search-secret".into();
        fs::write(path, serde_json::to_vec(&settings).unwrap()).unwrap();
        settings
    }
    #[test]
    fn migration_round_trips_keys_and_identity_without_plaintext_in_primary_backup_or_export() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let original = legacy(&path);
        let mut store = FakeStore::default();
        let migrated = load_at(&path, &mut store).unwrap();
        assert_eq!(migrated.vaults[0].secret_key, original.vaults[0].secret_key);
        assert_eq!(
            migrated.vaults[0].pairing_token,
            original.vaults[0].pairing_token
        );
        assert_eq!(
            migrated.ai.providers[0].api_key,
            original.ai.providers[0].api_key
        );
        assert_eq!(
            migrated.web_search.sources[0].api_key,
            original.web_search.sources[0].api_key
        );
        for bytes in [
            fs::read(&path).unwrap(),
            fs::read(storage::backup_path(&path)).unwrap(),
            serde_json::to_vec(&export_value(&migrated).unwrap()).unwrap(),
        ] {
            let value = String::from_utf8(bytes).unwrap();
            for secret in [
                &original.vaults[0].secret_key,
                &original.vaults[0].pairing_token,
                &original.ai.providers[0].api_key,
                &original.web_search.sources[0].api_key,
            ] {
                assert!(!value.contains(secret));
            }
        }
        let loaded = load_at(&path, &mut store).unwrap();
        assert_eq!(loaded.vaults[0].secret_key, original.vaults[0].secret_key);
    }
    #[test]
    fn unavailable_store_keeps_legacy_data_and_locked_migrated_vault_never_generates_a_new_identity(
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let original = legacy(&path);
        let before = fs::read(&path).unwrap();
        let mut store = FakeStore {
            locked: true,
            ..Default::default()
        };
        let failed = load_at(&path, &mut store).unwrap();
        assert!(!failed.credential_error.is_empty());
        assert_eq!(fs::read(&path).unwrap(), before);
        store.locked = false;
        load_at(&path, &mut store).unwrap();
        store.locked = true;
        let mut locked = load_at(&path, &mut store).unwrap();
        assert!(locked.vaults[0].credentials_locked);
        assert!(!locked.vaults[0].ensure_keys());
        assert!(locked.vaults[0].secret_key.is_empty());
        locked.theme = "dark".into();
        save_at(&path, &locked, &mut store).unwrap();
        store.locked = false;
        let restored = load_at(&path, &mut store).unwrap();
        assert_eq!(restored.theme, "dark");
        assert_eq!(restored.vaults[0].secret_key, original.vaults[0].secret_key);
    }
    #[test]
    fn failed_verification_preserves_plaintext_and_previous_credential_references_remain_recoverable(
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        legacy(&path);
        let before = fs::read(&path).unwrap();
        let mut store = FakeStore {
            corrupt_write: true,
            ..Default::default()
        };
        load_at(&path, &mut store).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        store.corrupt_write = false;
        let mut settings = load_at(&path, &mut store).unwrap();
        settings.ai.providers[0].api_key = "replacement-secret".into();
        save_at(&path, &settings, &mut store).unwrap();
        fs::write(&path, b"corrupted settings").unwrap();
        let recovered = load_at(&path, &mut store).unwrap();
        assert_eq!(recovered.ai.providers[0].api_key, "test-provider-secret");
    }
    #[test]
    #[ignore = "Requires an unlocked native credential store; uses an isolated synthetic entry"]
    fn native_store_round_trip() {
        let reference = format!("test-{:032x}", rand::rng().random::<u128>());
        let entry = keyring::Entry::new("dev.lowbloat.lownotes.tests", &reference).unwrap();
        entry.set_password("synthetic-test-value").unwrap();
        let result = entry.get_password();
        entry.delete_credential().unwrap();
        assert_eq!(result.unwrap(), "synthetic-test-value");
    }
}
