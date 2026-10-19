//! Immutable image blobs in one vault-local SQLite file. Sync transfers blobs, never the database.
use crate::vault::{Manifest, NoteMeta};
use anyhow::{bail, Context};
use image::{ImageDecoder, ImageEncoder, ImageFormat, ImageReader};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};

pub const MAX_INPUT_BYTES: usize = 50 * 1024 * 1024;
// Base64 transport stays below the sync protocol's 24 MiB packet limit.
pub const MAX_STORED_BYTES: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 32_000_000;
const DATABASE: &str = ".lownotes/images.sqlite3";
const SYNC_PREFIX: &str = ".lownotes/images/";
pub const LINK_PREFIX: &str = "lownotes-image:";

#[derive(Debug)]
pub struct StoredImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

pub fn image_name(value: &str) -> anyhow::Result<(&str, &str)> {
    let (id, extension) = value.split_once('.').context("errors.localImageInvalid")?;
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !matches!(extension, "png" | "jpg" | "gif" | "webp" | "bmp")
    {
        bail!("errors.localImageInvalid");
    }
    Ok((id, extension))
}

pub fn is_sync_image(path: &str) -> bool {
    path.starts_with(SYNC_PREFIX)
}
pub fn sync_name(path: &str) -> anyhow::Result<&str> {
    let name = path
        .strip_prefix(SYNC_PREFIX)
        .context("errors.localImageInvalid")?;
    image_name(name)?;
    Ok(name)
}
pub fn link_name(src: &str) -> anyhow::Result<&str> {
    let name = src
        .strip_prefix(LINK_PREFIX)
        .context("errors.localImageInvalid")?;
    image_name(name)?;
    Ok(name)
}

fn format(bytes: &[u8]) -> anyhow::Result<(&'static str, &'static str, ImageFormat)> {
    Ok(
        match image::guess_format(bytes).context("errors.imageUnsupported")? {
            ImageFormat::Png => ("png", "image/png", ImageFormat::Png),
            ImageFormat::Jpeg => ("jpg", "image/jpeg", ImageFormat::Jpeg),
            ImageFormat::Gif => ("gif", "image/gif", ImageFormat::Gif),
            ImageFormat::WebP => ("webp", "image/webp", ImageFormat::WebP),
            ImageFormat::Bmp => ("bmp", "image/bmp", ImageFormat::Bmp),
            _ => bail!("errors.imageUnsupported"),
        },
    )
}

fn database_path(root: &Path, create: bool) -> anyhow::Result<PathBuf> {
    let root = root.canonicalize()?;
    let dir = root.join(".lownotes");
    if create {
        fs::create_dir_all(&dir)?;
    }
    // A hidden directory redirected outside the vault must never expose arbitrary files.
    if !dir.canonicalize()?.starts_with(&root) {
        bail!("errors.pathEscape");
    }
    let path = root.join(DATABASE);
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            bail!("errors.pathEscape");
        }
    }
    Ok(path)
}

fn open(root: &Path, create: bool) -> anyhow::Result<Connection> {
    let path = database_path(root, create)?;
    let flags = if create {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(path, flags)?;
    db.busy_timeout(Duration::from_secs(10))?;
    if create {
        // DELETE journaling leaves one persistent file, unlike WAL's persistent sidecars.
        db.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS images (
                name TEXT PRIMARY KEY NOT NULL,
                data BLOB NOT NULL,
                CHECK(length(data) > 0 AND length(data) <= 16777216)
            );",
        )?;
    }
    Ok(db)
}

// Detect animation at container boundaries, not by searching compressed pixel bytes.
fn animated_container(bytes: &[u8], kind: ImageFormat) -> bool {
    let (mut offset, tag, little_endian) = match kind {
        ImageFormat::Png => (8usize, b"acTL".as_slice(), false),
        ImageFormat::WebP => (12usize, b"ANIM".as_slice(), true),
        _ => return false,
    };
    while let Some(header) = bytes.get(offset..offset.saturating_add(8)) {
        let (name, len) = if little_endian {
            (
                &header[..4],
                u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize,
            )
        } else {
            (
                &header[4..8],
                u32::from_be_bytes(header[..4].try_into().unwrap()) as usize,
            )
        };
        if name == tag {
            return true;
        }
        let overhead = if little_endian { 8 + len % 2 } else { 12 };
        let Some(next) = offset
            .checked_add(len)
            .and_then(|v| v.checked_add(overhead))
        else {
            break;
        };
        offset = next;
    }
    false
}

fn optimize_gif(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::Indexed);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        std::num::NonZeroU64::new(32_000_000).unwrap(),
    ));
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .context("errors.localImageInvalid")?;
    let first = decoder
        .read_next_frame()?
        .context("errors.localImageInvalid")?
        .clone();
    let mut output = Vec::new();
    {
        let mut encoder = gif::Encoder::new(
            &mut output,
            decoder.width(),
            decoder.height(),
            decoder.global_palette().unwrap_or(&[]),
        )?;
        encoder.set_repeat(decoder.repeat())?;
        let mut pixels = u64::from(first.width) * u64::from(first.height);
        encoder.write_frame(&first)?;
        while let Some(frame) = decoder.read_next_frame()? {
            pixels += u64::from(frame.width) * u64::from(frame.height);
            if pixels > 256_000_000 {
                bail!("errors.localImageDimensions");
            }
            encoder.write_frame(frame)?;
        }
    }
    Ok(output)
}

fn optimize(bytes: Vec<u8>) -> anyhow::Result<(Vec<u8>, &'static str)> {
    if bytes.len() > MAX_INPUT_BYTES {
        bail!("errors.localImageTooLarge");
    }
    let (extension, _, kind) = format(&bytes)?;
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), kind)
        .into_dimensions()
        .context("errors.localImageInvalid")?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        bail!("errors.localImageDimensions");
    }
    let (candidate, candidate_extension) = if kind == ImageFormat::Gif {
        (optimize_gif(&bytes)?, "gif")
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), kind);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().context("errors.localImageInvalid")?;
        let profile = decoder.icc_profile().ok().flatten();
        let orientation = decoder.orientation().ok();
        let mut decoded =
            image::DynamicImage::from_decoder(decoder).context("errors.localImageInvalid")?;
        if let Some(orientation) = orientation {
            decoded.apply_orientation(orientation);
        }
        if animated_container(&bytes, kind) {
            // APNG/animated WebP already compress their frames. Never flatten the animation.
            (bytes.clone(), extension)
        } else {
            let rgba = decoded.to_rgba8();
            let mut output = Vec::new();
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut output);
            if let Some(profile) = profile {
                encoder.set_icc_profile(profile)?;
            }
            encoder.write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            (output, "webp")
        }
    };
    let result = if candidate.len() < bytes.len() {
        (candidate, candidate_extension)
    } else {
        (bytes, extension)
    };
    if result.0.len() > MAX_STORED_BYTES {
        bail!("errors.localImageStoredTooLarge");
    }
    Ok(result)
}

pub fn save_pasted(root: &Path, bytes: Vec<u8>) -> anyhow::Result<String> {
    let (bytes, extension) = optimize(bytes)?;
    let name = format!("{}.{extension}", blake3::hash(&bytes).to_hex());
    insert(root, &name, &bytes)?;
    Ok(format!("{LINK_PREFIX}{name}"))
}

fn validate_blob(name: &str, bytes: &[u8]) -> anyhow::Result<&'static str> {
    let (id, extension) = image_name(name)?;
    if bytes.is_empty()
        || bytes.len() > MAX_STORED_BYTES
        || blake3::hash(bytes).to_hex().as_str() != id
    {
        bail!("errors.localImageInvalid");
    }
    let (actual_extension, mime, _) = format(bytes)?;
    if extension != actual_extension {
        bail!("errors.localImageInvalid");
    }
    Ok(mime)
}

pub fn insert(root: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<bool> {
    validate_blob(name, bytes)?;
    let db = open(root, true).context("errors.localImageSave")?;
    Ok(db.execute(
        "INSERT OR IGNORE INTO images(name,data) VALUES (?1,?2)",
        params![name, bytes],
    )? > 0)
}

pub fn load(root: &Path, name: &str) -> anyhow::Result<StoredImage> {
    image_name(name)?;
    let db = open(root, false).context("errors.localImageNotFound")?;
    let bytes: Vec<u8> = db
        .query_row("SELECT data FROM images WHERE name=?1", [name], |r| {
            r.get(0)
        })
        .optional()?
        .context("errors.localImageNotFound")?;
    let mime = validate_blob(name, &bytes)?;
    Ok(StoredImage { bytes, mime })
}

pub fn manifest(root: &Path) -> anyhow::Result<Manifest> {
    let mut manifest = Manifest::new();
    if !root.join(DATABASE).exists() {
        return Ok(manifest);
    }
    let db = open(root, false)?;
    let mut query = db.prepare("SELECT name,length(data) FROM images ORDER BY name")?;
    for row in query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (name, size) = row?;
        let size = u64::try_from(size)?;
        let (id, _) = image_name(&name)?;
        let path = format!("{SYNC_PREFIX}{name}");
        manifest.insert(
            path.clone(),
            NoteMeta {
                path,
                size,
                hash: id.into(),
                modified_ms: 0,
            },
        );
    }
    Ok(manifest)
}

/// Paths contain a local vault ID plus a validated content hash, never a filesystem path.
pub fn protocol_response(
    settings: &crate::config::AppSettings,
    path: &str,
) -> tauri::http::Response<Vec<u8>> {
    let image = (|| -> anyhow::Result<StoredImage> {
        let decoded = percent_encoding::percent_decode_str(path).decode_utf8()?;
        let path = decoded.as_ref();
        let (vault_id, name) = path
            .strip_prefix('/')
            .context("errors.localImageInvalid")?
            .split_once('/')
            .context("errors.localImageInvalid")?;
        let vault = settings
            .vaults
            .iter()
            .find(|v| v.id == vault_id)
            .context("errors.noVaultSelected")?;
        load(&vault.path, name)
    })();
    match image {
        Ok(image) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", image.mime)
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .header("Access-Control-Allow-Origin", "*")
            .body(image.bytes)
            .unwrap(),
        Err(_) => tauri::http::Response::builder()
            .status(404)
            .header("Cache-Control", "no-store")
            .body(Vec::new())
            .unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppSettings, VaultConfig};

    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_fn(80, 40, |x, y| {
            image::Rgba([x as u8, y as u8, 100, if x < 40 { 0 } else { 255 }])
        });
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut bytes,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        )
        .write_image(&image, 80, 40, image::ExtendedColorType::Rgba8)
        .unwrap();
        bytes
    }

    #[test]
    fn compressed_images_round_trip_losslessly_and_deduplicate_in_one_hidden_database() {
        let dir = tempfile::tempdir().unwrap();
        let original = png();
        let link = save_pasted(dir.path(), original.clone()).unwrap();
        let name = link_name(&link).unwrap();
        let stored = load(dir.path(), name).unwrap();
        assert!(stored.bytes.len() < original.len());
        assert_eq!(stored.mime, "image/webp");
        assert_eq!(
            image::load_from_memory(&stored.bytes).unwrap().to_rgba8(),
            image::load_from_memory(&original).unwrap().to_rgba8()
        );
        assert_eq!(save_pasted(dir.path(), original).unwrap(), link);
        assert_eq!(manifest(dir.path()).unwrap().len(), 1);
        assert_eq!(
            fs::read_dir(dir.path().join(".lownotes")).unwrap().count(),
            1
        );
        fs::write(dir.path().join("note.md"), format!("![image]({link})")).unwrap();
        let items = crate::vault::list_vault_items(dir.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "note.md");
        assert!(crate::vault::build_manifest(dir.path())
            .unwrap()
            .keys()
            .any(|key| is_sync_image(key)));
        assert!(!crate::vault::build_manifest(dir.path())
            .unwrap()
            .contains_key(DATABASE));
        crate::vault::rename_item(dir.path(), "note.md", "folder/renamed.md").unwrap();
        assert!(load(dir.path(), name).is_ok());
    }

    #[test]
    fn gif_optimization_preserves_frames_timing_transparency_and_repeat() {
        let mut bytes = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 2, 1, &[255, 0, 0, 0, 255, 0]).unwrap();
            encoder.set_repeat(gif::Repeat::Finite(3)).unwrap();
            for color in [0, 1] {
                let frame = gif::Frame {
                    width: 2,
                    height: 1,
                    delay: 7,
                    transparent: Some(1),
                    dispose: gif::DisposalMethod::Background,
                    buffer: std::borrow::Cow::Owned(vec![color, color]),
                    ..Default::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        let (optimized, extension) = optimize(bytes).unwrap();
        assert_eq!(extension, "gif");
        let mut decoder = gif::DecodeOptions::new()
            .read_info(Cursor::new(optimized))
            .unwrap();
        for color in [0, 1] {
            let frame = decoder.read_next_frame().unwrap().unwrap();
            assert_eq!(frame.buffer.as_ref(), &[color, color]);
            assert_eq!(frame.delay, 7);
            assert_eq!(frame.transparent, Some(1));
            assert_eq!(frame.dispose, gif::DisposalMethod::Background);
        }
        assert!(decoder.read_next_frame().unwrap().is_none());
        assert_eq!(decoder.repeat(), gif::Repeat::Finite(3));
    }

    #[test]
    fn rejects_bad_images_paths_and_corrupt_sync_blobs_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(save_pasted(dir.path(), b"not an image".to_vec()).is_err());
        for name in [
            "../../outside.png",
            "x.png",
            &format!("{}.exe", "a".repeat(64)),
            &format!("{}.png?secret", "a".repeat(64)),
        ] {
            assert!(load(dir.path(), name).is_err());
        }
        let data = png();
        let correct = format!("{}.png", blake3::hash(&data).to_hex());
        assert!(insert(dir.path(), &format!("{}.png", "0".repeat(64)), &data).is_err());
        assert!(!dir.path().join(DATABASE).exists());
        assert!(insert(dir.path(), &correct, &data).unwrap());
        assert!(!insert(dir.path(), &correct, &data).unwrap());
        assert_eq!(load(dir.path(), &correct).unwrap().bytes, data);
    }

    #[tokio::test]
    async fn local_protocol_and_export_read_the_database_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let link = save_pasted(dir.path(), png()).unwrap();
        let name = link_name(&link).unwrap();
        let mut settings = AppSettings::default();
        let vault = VaultConfig::new(dir.path().to_path_buf(), None);
        let vault_id = vault.id.clone();
        settings.vaults.push(vault);
        let response = protocol_response(&settings, &format!("/{vault_id}/{name}"));
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "image/webp");
        assert_eq!(
            protocol_response(&settings, &format!("/{vault_id}%2F{name}")).body(),
            response.body()
        );
        let exported = crate::export_images::load_image(dir.path(), "note.md", &link)
            .await
            .unwrap();
        assert_eq!(response.body(), &exported.bytes);
        assert_eq!(exported.mime_type, "image/webp");
        for path in [
            format!("/unknown/{name}"),
            format!("/{vault_id}/../settings.json"),
            format!("/{vault_id}/{name}/secret"),
        ] {
            let response = protocol_response(&settings, &path);
            assert_eq!(response.status(), 404);
            assert!(response.body().is_empty());
        }
    }

    #[test]
    fn simultaneous_inserts_keep_both_images() {
        let dir = tempfile::tempdir().unwrap();
        let first = png();
        let mut second = first.clone();
        // Valid PNG ancillary bytes change the immutable blob ID without affecting its file signature.
        second.extend_from_slice(b"second image");
        std::thread::scope(|scope| {
            for bytes in [&first, &second] {
                let root = dir.path();
                scope.spawn(move || {
                    let name = format!("{}.png", blake3::hash(bytes).to_hex());
                    insert(root, &name, bytes).unwrap();
                });
            }
        });
        assert_eq!(manifest(dir.path()).unwrap().len(), 2);
    }
}
