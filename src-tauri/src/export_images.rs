use anyhow::{bail, Context};
use serde::Serialize;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;

/// Only globally routable addresses. Notes can come from peers or the
/// assistant, so exporting one must not reach loopback, LAN or metadata hosts.
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let segments = v6.segments();
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (segments[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
                || (segments[0] == 0x0064 && segments[1] == 0xff9b) // NAT64 maps to IPv4
                || (segments[0] == 0x2002)) // 6to4 embeds IPv4
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_broadcast()
        || ip.is_documentation() || ip.is_unspecified() || ip.is_multicast()
        || a == 0 || a >= 240
        || (a == 100 && (64..128).contains(&b)) // shared address space
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (18..20).contains(&b))) // benchmarking
}

/// Remote export images must use HTTPS and, when the host is a literal
/// address, a public one. Host names are checked by `PublicResolver`.
fn allowed_remote_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let literal = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = literal.parse::<IpAddr>() {
        return is_public_ip(ip);
    }
    let host = host.to_ascii_lowercase();
    host != "localhost" && !host.ends_with(".localhost")
}

/// Resolves host names and drops non-public addresses, so redirects and DNS
/// answers cannot point the request at the local network.
struct PublicResolver;

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|addr| is_public_ip(addr.ip()))
                .collect();
            if addrs.is_empty() {
                return Err("export image host does not resolve to a public address".into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Built once. Unlike `crate::http::cached`, a build failure is an error here:
/// falling back to a default client would drop the public-address resolver.
fn remote_client() -> anyhow::Result<&'static reqwest::Client> {
    static EXPORT_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(client) = EXPORT_CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(20))
        .no_proxy()
        .https_only(true)
        .dns_resolver(Arc::new(PublicResolver))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS || !allowed_remote_url(attempt.url()) {
                attempt.error("export image redirect refused")
            } else {
                attempt.follow()
            }
        }))
        .build()?;
    Ok(EXPORT_CLIENT.get_or_init(|| client))
}

#[derive(Serialize)]
pub struct ExportImageData {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

fn decode_path(value: &str) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    let source = value.as_bytes();
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let hex = source
                .get(index + 1..index + 3)
                .context("export.imageFailed")?;
            bytes.push(u8::from_str_radix(std::str::from_utf8(hex)?, 16)?);
            index += 3;
        } else {
            bytes.push(source[index]);
            index += 1;
        }
    }
    Ok(String::from_utf8(bytes)?)
}

fn local_image_path(root: &Path, note_path: &str, src: &str) -> anyhow::Result<PathBuf> {
    let source = decode_path(src.split(['?', '#']).next().unwrap_or_default())?.replace('\\', "/");
    let note = note_path.replace('\\', "/");
    if source.contains(':') || note.contains(':') {
        bail!("export.imageFailed");
    }
    let parent = if source.starts_with('/') {
        ""
    } else {
        note.rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("")
    };
    let mut components = Vec::new();
    for part in parent
        .split('/')
        .chain(source.trim_start_matches('/').split('/'))
    {
        match part {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    bail!("export.imageFailed");
                }
            }
            value => components.push(value),
        }
    }
    let canonical_root = root.canonicalize()?;
    let candidate = components
        .iter()
        .fold(canonical_root.clone(), |path, part| path.join(part))
        .canonicalize()?;
    if !candidate.starts_with(&canonical_root) || !candidate.is_file() {
        bail!("export.imageFailed");
    }
    Ok(candidate)
}

fn image_data(bytes: Vec<u8>) -> anyhow::Result<ExportImageData> {
    if bytes.len() > MAX_IMAGE_BYTES {
        bail!("export.imageFailed");
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else if std::str::from_utf8(&bytes)
        .map(|text| {
            text.trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with("<svg")
                || (text.trim_start().starts_with("<?xml") && text.contains("<svg"))
        })
        .unwrap_or(false)
    {
        "image/svg+xml"
    } else {
        bail!("export.imageFailed");
    };
    Ok(ExportImageData {
        bytes,
        mime_type: mime.into(),
    })
}

pub async fn load_image(
    root: &Path,
    note_path: &str,
    src: &str,
) -> anyhow::Result<ExportImageData> {
    if src.starts_with(crate::local_images::LINK_PREFIX) {
        let name = crate::local_images::link_name(src)?;
        let image = crate::local_images::load(root, name)?;
        return Ok(ExportImageData {
            bytes: image.bytes,
            mime_type: image.mime.into(),
        });
    }
    let bytes = if src.starts_with("https://") || src.starts_with("http://") {
        let url = reqwest::Url::parse(src)?;
        if !allowed_remote_url(&url) {
            bail!("export.imageFailed");
        }
        let mut response = remote_client()?.get(url).send().await?.error_for_status()?;
        if response.content_length().unwrap_or(0) > MAX_IMAGE_BYTES as u64 {
            bail!("export.imageFailed");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                bail!("export.imageFailed");
            }
            bytes.extend_from_slice(&chunk);
        }
        bytes
    } else {
        let path = local_image_path(root, note_path, src)?;
        if std::fs::metadata(&path)?.len() > MAX_IMAGE_BYTES as u64 {
            bail!("export.imageFailed");
        }
        std::fs::read(path)?
    };
    image_data(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_note_relative_images_and_rejects_vault_escape() {
        let root = std::env::temp_dir().join(format!("lownotes-images-{}", rand::random::<u64>()));
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/image name.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let expected = root.join("assets/image name.png").canonicalize().unwrap();
        assert_eq!(
            local_image_path(&root, "notes/note.md", "../assets/image%20name.png").unwrap(),
            expected
        );
        assert_eq!(
            local_image_path(&root, "notes/note.md", "/assets/image%20name.png").unwrap(),
            expected
        );
        assert!(local_image_path(&root, "note.md", "../outside.png").is_err());
        assert!(local_image_path(&root, "note.md", "C:/outside.png").is_err());
        assert!(local_image_path(&root, "note.md", "%2e%2e/outside.png").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn remote_images_require_https_and_public_hosts() {
        let allowed = |value: &str| allowed_remote_url(&reqwest::Url::parse(value).unwrap());
        assert!(allowed("https://example.org/a.png"));
        assert!(allowed("https://93.184.216.34/a.png"));
        for refused in [
            "http://example.org/a.png",
            "https://localhost/a.png",
            "https://app.localhost/a.png",
            "https://127.0.0.1/a.png",
            "https://10.0.0.5/a.png",
            "https://192.168.1.1/a.png",
            "https://169.254.169.254/latest",
            "https://100.64.0.1/a.png",
            "https://[::1]/a.png",
            "https://[fd00::1]/a.png",
            "https://[fe80::1]/a.png",
            "https://[::ffff:127.0.0.1]/a.png",
            "https://user:pass@example.org/a.png",
            "https://0.0.0.0/a.png",
        ] {
            assert!(!allowed(refused), "{refused} should be refused");
        }
        assert!(is_public_ip("2606:4700::1111".parse().unwrap()));
        assert!(!is_public_ip("64:ff9b::7f00:1".parse().unwrap()));
    }

    #[tokio::test]
    async fn loading_refuses_local_network_targets_without_connecting() {
        let root = tempfile::tempdir().unwrap();
        for src in [
            "http://example.org/a.png",
            "https://127.0.0.1:1/a.png",
            "https://[::1]:1/a.png",
        ] {
            assert!(
                load_image(root.path(), "note.md", src).await.is_err(),
                "{src}"
            );
        }
    }

    #[test]
    fn validates_image_data_and_size() {
        assert_eq!(
            image_data(b"\x89PNG\r\n\x1a\n".to_vec()).unwrap().mime_type,
            "image/png"
        );
        assert_eq!(
            image_data(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".to_vec())
                .unwrap()
                .mime_type,
            "image/svg+xml"
        );
        assert!(image_data(b"secret text".to_vec()).is_err());
        assert!(image_data(vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    }
}
