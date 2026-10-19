use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

const UPDATE_URL: &str =
    "https://github.com/LowBloat/LowNotes/releases/latest/download/latest.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Internal,
    Appimage,
    Arch,
    Deb,
    Rpm,
    Flatpak,
    Snap,
    Portable,
    LinuxPackage,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdatePolicy {
    pub channel: UpdateChannel,
    pub can_install: bool,
    pub updater_target: Option<String>,
}

impl UpdatePolicy {
    fn new(channel: UpdateChannel, appimage_writable: bool, arch: &str) -> Self {
        let can_install = channel == UpdateChannel::Internal
            || (channel == UpdateChannel::Appimage && appimage_writable);
        Self {
            channel,
            can_install,
            updater_target: (channel == UpdateChannel::Appimage && can_install)
                .then(|| format!("linux-{arch}-appimage")),
        }
    }

    pub fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            detect_linux_policy()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::new(UpdateChannel::Internal, false, std::env::consts::ARCH)
        }
    }
}

// Keep classification separate from probing so all Linux formats can be tested
// on any host, including ownership taking precedence over an AppImage wrapper.
#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
struct LinuxInstallation {
    flatpak: bool,
    snap: bool,
    owner: Option<UpdateChannel>,
    bundle: Option<tauri::utils::config::BundleType>,
    appimage: bool,
    system_path: bool,
}

#[cfg(any(target_os = "linux", test))]
fn linux_channel(installation: &LinuxInstallation) -> UpdateChannel {
    use tauri::utils::config::BundleType;
    if installation.flatpak {
        UpdateChannel::Flatpak
    } else if installation.snap {
        UpdateChannel::Snap
    } else if let Some(owner) = installation.owner {
        owner
    } else {
        match installation.bundle {
            Some(BundleType::Deb) => UpdateChannel::Deb,
            Some(BundleType::Rpm) => UpdateChannel::Rpm,
            Some(BundleType::AppImage) => UpdateChannel::Appimage,
            _ if installation.appimage => UpdateChannel::Appimage,
            _ if installation.system_path => UpdateChannel::LinuxPackage,
            _ => UpdateChannel::Portable,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn detect_linux_policy() -> UpdatePolicy {
    use std::{
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    let appimage = std::env::var_os("APPIMAGE")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let executable = appimage.clone().or_else(|| std::env::current_exe().ok());
    let flatpak = std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists();
    let snap = std::env::var_os("SNAP").is_some();
    let mut owner = None;
    if !flatpak && !snap {
        if let Some(path) = &executable {
            // Query ownership, not the host distribution: a portable binary on
            // Arch must not be classified as a pacman package.
            for (channel, program, args) in [
                (UpdateChannel::Arch, "pacman", vec!["-Qoq"]),
                (UpdateChannel::Deb, "dpkg-query", vec!["-S"]),
                (UpdateChannel::Rpm, "rpm", vec!["-qf"]),
            ] {
                if Command::new(program)
                    .args(args)
                    .arg(path)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success())
                {
                    owner = Some(channel);
                    break;
                }
            }
        }
    }
    let channel = linux_channel(&LinuxInstallation {
        flatpak,
        snap,
        owner,
        bundle: tauri::utils::platform::bundle_type(),
        appimage: appimage.is_some(),
        system_path: executable.as_ref().is_some_and(|path| {
            ["/usr", "/opt", "/nix", "/app", "/snap", "/var/lib/flatpak"]
                .iter()
                .any(|prefix| path.starts_with(prefix))
        }),
    });
    // Replacing an AppImage requires creating a backup in its parent directory.
    // Probe this only for genuine unmanaged AppImages, never for native packages.
    let writable = channel == UpdateChannel::Appimage
        && executable
            .as_ref()
            .and_then(|path| path.parent())
            .is_some_and(|parent| {
                tempfile::Builder::new()
                    .prefix(".lownotes-update-check-")
                    .tempfile_in(parent)
                    .is_ok()
            });
    UpdatePolicy::new(channel, writable, std::env::consts::ARCH)
}

#[derive(Debug, Deserialize)]
struct ReleaseMetadata {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    downloads: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct ReleaseNotice {
    version: String,
    body: String,
    download_url: Option<String>,
}

fn release_notice(
    metadata: ReleaseMetadata,
    current: &semver::Version,
    policy: &UpdatePolicy,
    arch: &str,
) -> Result<Option<ReleaseNotice>, String> {
    let version = semver::Version::parse(&metadata.version).map_err(|error| error.to_string())?;
    if version <= *current {
        return Ok(None);
    }
    let format = match policy.channel {
        UpdateChannel::Arch => Some("arch"),
        UpdateChannel::Deb => Some("deb"),
        UpdateChannel::Rpm => Some("rpm"),
        UpdateChannel::Portable => Some("portable"),
        UpdateChannel::Appimage => Some("appimage"),
        _ => None,
    };
    let prefix = format!("https://github.com/LowBloat/LowNotes/releases/download/v{version}/");
    let download_url = format
        .and_then(|format| metadata.downloads.get(&format!("linux-{arch}-{format}")))
        .filter(|url| url.starts_with(&prefix))
        .cloned();
    Ok(Some(ReleaseNotice {
        version: version.to_string(),
        body: metadata.notes,
        download_url,
    }))
}

#[tauri::command]
pub fn get_update_policy(policy: State<'_, UpdatePolicy>) -> UpdatePolicy {
    policy.inner().clone()
}

#[tauri::command]
pub async fn check_external_update(
    app: AppHandle,
    policy: State<'_, UpdatePolicy>,
) -> Result<Option<ReleaseNotice>, String> {
    if policy.can_install {
        return Ok(None);
    }
    let metadata = crate::http::client()
        .get(UPDATE_URL)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<ReleaseMetadata>()
        .await
        .map_err(|error| error.to_string())?;
    release_notice(
        metadata,
        &app.package_info().version,
        &policy,
        std::env::consts::ARCH,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::utils::config::BundleType;

    #[test]
    fn native_packages_and_sandboxes_never_install_internally() {
        for channel in [
            UpdateChannel::Arch,
            UpdateChannel::Deb,
            UpdateChannel::Rpm,
            UpdateChannel::Flatpak,
            UpdateChannel::Snap,
            UpdateChannel::Portable,
            UpdateChannel::LinuxPackage,
        ] {
            let policy = UpdatePolicy::new(channel, true, "x86_64");
            assert!(!policy.can_install);
            assert!(policy.updater_target.is_none());
        }
        let appimage = UpdatePolicy::new(UpdateChannel::Appimage, true, "x86_64");
        assert!(appimage.can_install);
        assert_eq!(
            appimage.updater_target.as_deref(),
            Some("linux-x86_64-appimage")
        );
        assert!(!UpdatePolicy::new(UpdateChannel::Appimage, false, "x86_64").can_install);
        assert!(UpdatePolicy::new(UpdateChannel::Internal, false, "x86_64").can_install);
    }

    #[test]
    fn detects_linux_packages_without_relying_on_distribution() {
        for (bundle, expected) in [
            (BundleType::Deb, UpdateChannel::Deb),
            (BundleType::Rpm, UpdateChannel::Rpm),
            (BundleType::AppImage, UpdateChannel::Appimage),
        ] {
            assert_eq!(
                linux_channel(&LinuxInstallation {
                    bundle: Some(bundle),
                    ..Default::default()
                }),
                expected
            );
        }
        assert_eq!(
            linux_channel(&LinuxInstallation::default()),
            UpdateChannel::Portable
        );
        assert_eq!(
            linux_channel(&LinuxInstallation {
                system_path: true,
                ..Default::default()
            }),
            UpdateChannel::LinuxPackage
        );
        assert_eq!(
            linux_channel(&LinuxInstallation {
                appimage: true,
                ..Default::default()
            }),
            UpdateChannel::Appimage
        );
    }

    #[test]
    fn package_ownership_and_sandboxes_override_appimage_markers() {
        for owner in [UpdateChannel::Arch, UpdateChannel::Deb, UpdateChannel::Rpm] {
            assert_eq!(
                linux_channel(&LinuxInstallation {
                    owner: Some(owner),
                    bundle: Some(BundleType::AppImage),
                    appimage: true,
                    ..Default::default()
                }),
                owner
            );
        }
        assert_eq!(
            linux_channel(&LinuxInstallation {
                flatpak: true,
                owner: Some(UpdateChannel::Deb),
                ..Default::default()
            }),
            UpdateChannel::Flatpak
        );
        assert_eq!(
            linux_channel(&LinuxInstallation {
                snap: true,
                bundle: Some(BundleType::Deb),
                ..Default::default()
            }),
            UpdateChannel::Snap
        );
    }

    fn metadata(version: &str) -> ReleaseMetadata {
        ReleaseMetadata {
            version: version.into(),
            notes: "Release notes".into(),
            downloads: HashMap::new(),
        }
    }

    #[test]
    fn checks_versions_and_old_manifests_without_appimage_fallback() {
        let current = semver::Version::parse("0.2.6").unwrap();
        let policy = UpdatePolicy::new(UpdateChannel::Arch, false, "x86_64");
        for version in ["0.2.5", "0.2.6", "0.2.6-beta.1"] {
            assert!(
                release_notice(metadata(version), &current, &policy, "x86_64")
                    .unwrap()
                    .is_none()
            );
        }
        let notice = release_notice(metadata("0.2.7"), &current, &policy, "x86_64")
            .unwrap()
            .unwrap();
        assert_eq!(notice.version, "0.2.7");
        assert!(notice.download_url.is_none());
        assert!(release_notice(metadata("invalid"), &current, &policy, "x86_64").is_err());
    }

    #[test]
    fn download_links_match_installed_format_and_architecture() {
        let current = semver::Version::parse("0.2.6").unwrap();
        for (channel, format) in [
            (UpdateChannel::Arch, "arch"),
            (UpdateChannel::Deb, "deb"),
            (UpdateChannel::Rpm, "rpm"),
            (UpdateChannel::Portable, "portable"),
            (UpdateChannel::Appimage, "appimage"),
        ] {
            let mut release = metadata("0.2.7");
            let url =
                format!("https://github.com/LowBloat/LowNotes/releases/download/v0.2.7/{format}");
            release
                .downloads
                .insert(format!("linux-x86_64-{format}"), url.clone());
            release.downloads.insert(
                "linux-x86_64-appimage".into(),
                "https://example.com/untrusted.AppImage".into(),
            );
            if channel == UpdateChannel::Appimage {
                release
                    .downloads
                    .insert("linux-x86_64-appimage".into(), url.clone());
            }
            let notice = release_notice(
                release,
                &current,
                &UpdatePolicy::new(channel, false, "x86_64"),
                "x86_64",
            )
            .unwrap()
            .unwrap();
            assert_eq!(notice.download_url, Some(url));
        }
        let mut release = metadata("0.2.7");
        release.downloads.insert(
            "linux-x86_64-deb".into(),
            "https://example.com/pkg.deb".into(),
        );
        assert!(release_notice(
            release,
            &current,
            &UpdatePolicy::new(UpdateChannel::Deb, false, "x86_64"),
            "x86_64"
        )
        .unwrap()
        .unwrap()
        .download_url
        .is_none());
    }
}
