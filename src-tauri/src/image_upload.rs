use std::time::Duration;

use crate::{
    commands::AppState,
    config::{AppSettings, ImageUploadSettings},
};
use reqwest::multipart::{Form, Part};
use tauri::ipc::{InvokeBody, Request};
use tauri::State;

const ENDPOINT: &str = "https://catbox.moe/user/api.php";
const IMGUR_ENDPOINT: &str = "https://api.imgur.com/3/image";
// Client IDs identify public applications; this is not a Client Secret.
const IMGUR_CLIENT_ID: &str = "0f778c0aada9495";
const MAX_IMGUR_BYTES: usize = 10 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 200 * 1024 * 1024;
const MAX_GIF_BYTES: usize = 20 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Provider {
    Local,
    Catbox,
    Imgur,
}

impl Provider {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "local" => Ok(Self::Local),
            "catbox" => Ok(Self::Catbox),
            "imgur" => Ok(Self::Imgur),
            _ => Err("errors.imageProvider".into()),
        }
    }
    fn error(self, kind: &str) -> String {
        format!(
            "errors.{}{kind}",
            if self == Self::Imgur {
                "imgur"
            } else {
                "catbox"
            }
        )
    }
    fn image_type(self, bytes: &[u8]) -> Result<(&'static str, &'static str), String> {
        if self == Self::Local && bytes.len() > crate::local_images::MAX_INPUT_BYTES {
            return Err("errors.localImageTooLarge".into());
        }
        if self == Self::Imgur && bytes.len() > MAX_IMGUR_BYTES {
            return Err("errors.imgurTooLarge".into());
        }
        let image = image_type(bytes)?;
        if self == Self::Imgur && !matches!(image.0, "png" | "jpg" | "gif") {
            return Err("errors.imgurUnsupported".into());
        }
        Ok(image)
    }
}

fn client_id(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(IMGUR_CLIENT_ID);
    }
    if value.len() > 128 || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("errors.imgurClientId".into());
    }
    Ok(value)
}

#[derive(Debug)]
struct UploadedImage {
    link: String,
    deletehash: Option<String>,
}

fn image_type(bytes: &[u8]) -> Result<(&'static str, &'static str), String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("errors.imageTooLarge".into());
    }
    let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ("png", "image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        ("jpg", "image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        if bytes.len() > MAX_GIF_BYTES {
            return Err("errors.imageGifTooLarge".into());
        }
        ("gif", "image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        ("webp", "image/webp")
    } else if bytes.starts_with(b"BM") {
        ("bmp", "image/bmp")
    } else {
        return Err("errors.imageUnsupported".into());
    };
    Ok(format)
}

fn validate_response(body: &str) -> Result<String, String> {
    validate_link(Provider::Catbox, body)
}

fn validate_link(provider: Provider, body: &str) -> Result<String, String> {
    let link = body.trim();
    let url = reqwest::Url::parse(link).map_err(|_| provider.error("Response"))?;
    let file = url.path().strip_prefix('/').unwrap_or_default();
    let valid_file = file.split_once('.').is_some_and(|(id, extension)| {
        !id.is_empty()
            && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && matches!(extension, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
            && (provider == Provider::Catbox || matches!(extension, "png" | "jpg" | "jpeg" | "gif"))
    });
    if url.scheme() != "https"
        || url.host_str()
            != Some(if provider == Provider::Imgur {
                "i.imgur.com"
            } else {
                "files.catbox.moe"
            })
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_file
    {
        return Err(provider.error("Response"));
    }
    Ok(url.to_string())
}

fn imgur_response(body: &str) -> Result<UploadedImage, String> {
    let response: serde_json::Value =
        serde_json::from_str(body).map_err(|_| Provider::Imgur.error("Response"))?;
    if response["success"] != true {
        return Err(Provider::Imgur.error("Upload"));
    }
    let link = response["data"]["link"]
        .as_str()
        .ok_or_else(|| Provider::Imgur.error("Response"))?;
    let deletehash = response["data"]["deletehash"]
        .as_str()
        .filter(|hash| {
            !hash.is_empty() && hash.len() <= 128 && hash.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .ok_or_else(|| Provider::Imgur.error("Response"))?;
    Ok(UploadedImage {
        link: validate_link(Provider::Imgur, link)?,
        deletehash: Some(deletehash.into()),
    })
}

#[cfg(test)]
async fn upload(endpoint: &str, bytes: Vec<u8>) -> Result<String, String> {
    Ok(upload_to(Provider::Catbox, endpoint, bytes, "").await?.link)
}

async fn upload_to(
    provider: Provider,
    endpoint: &str,
    bytes: Vec<u8>,
    imgur_id: &str,
) -> Result<UploadedImage, String> {
    let (extension, mime) = provider.image_type(&bytes)?;
    let imgur_id = if provider == Provider::Imgur {
        client_id(imgur_id)?
    } else {
        ""
    };
    let part = Part::bytes(bytes)
        .file_name(format!("image.{extension}"))
        .mime_str(mime)
        .map_err(|_| "errors.imageInvalid".to_string())?;
    // Omitting userhash selects anonymous uploads; no application keys are needed.
    let form = match provider {
        Provider::Local => return Err("errors.imageProvider".into()),
        Provider::Catbox => Form::new()
            .text("reqtype", "fileupload")
            .part("fileToUpload", part),
        Provider::Imgur => Form::new().text("type", "file").part("image", part),
    };
    static UPLOAD_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = crate::http::cached(&UPLOAD_CLIENT, |builder| builder
        // Catbox closes upload connections when the client has no User-Agent.
        .user_agent(concat!("LowNotes/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none()));
    let mut request = client.post(endpoint).multipart(form);
    if provider == Provider::Imgur {
        request = request.header(
            reqwest::header::AUTHORIZATION,
            format!("Client-ID {imgur_id}"),
        );
    }
    let mut response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            provider.error("Timeout")
        } else {
            provider.error("Connection")
        }
    })?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => provider.error("Rejected"),
            413 => {
                if provider == Provider::Imgur {
                    "errors.imgurTooLarge".into()
                } else {
                    "errors.imageTooLarge".into()
                }
            }
            429 => provider.error("RateLimit"),
            _ => provider.error("Upload"),
        }
        .into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| provider.error("Response"))?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(provider.error("Response"));
        }
        body.extend_from_slice(&chunk);
    }
    let body = std::str::from_utf8(&body).map_err(|_| provider.error("Response"))?;
    match provider {
        Provider::Local => Err("errors.imageProvider".into()),
        Provider::Catbox => Ok(UploadedImage {
            link: validate_response(body)?,
            deletehash: None,
        }),
        Provider::Imgur => imgur_response(body),
    }
}

#[tauri::command]
pub async fn upload_clipboard_image(
    request: Request<'_>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("errors.imageInvalid".into());
    };
    // Validate before cloning. Binary IPC avoids base64 expansion for larger images.
    let settings = state.settings.read().image_upload.clone();
    // Freeze the provider at paste time, including retries after a settings change.
    let name = request
        .headers()
        .get("x-upload-provider")
        .map(|v| v.to_str())
        .transpose()
        .map_err(|_| "errors.imageProvider".to_string())?
        .unwrap_or(&settings.provider);
    let provider = Provider::parse(name)?;
    provider.image_type(bytes)?;
    if provider == Provider::Local {
        let vault_id = request
            .headers()
            .get("x-vault-id")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| "errors.noVaultSelected".to_string())?;
        let vault = state
            .settings
            .read()
            .vaults
            .iter()
            .find(|v| v.id == vault_id)
            .cloned()
            .ok_or_else(|| "errors.noVaultSelected".to_string())?;
        let bytes = bytes.clone();
        let root = vault.path.clone();
        let link =
            tokio::task::spawn_blocking(move || crate::local_images::save_pasted(&root, bytes))
                .await
                .map_err(|_| "errors.localImageSave".to_string())?
                .map_err(|error| {
                    let message = error.to_string();
                    if message.starts_with("errors.") {
                        message
                    } else {
                        "errors.localImageSave".into()
                    }
                })?;
        // Only the active vault has a running network service.
        if state.settings.read().active_vault_id.as_deref() == Some(vault_id) {
            if let Some(network) = state.network.read().as_ref() {
                network.sync_now();
            }
        }
        return Ok(link);
    }
    let endpoint = if provider == Provider::Imgur {
        IMGUR_ENDPOINT
    } else {
        ENDPOINT
    };
    let image = upload_to(provider, endpoint, bytes.clone(), &settings.imgur_client_id).await?;
    if let Some(hash) = &image.deletehash {
        // Keep anonymous deletion credentials locally, outside notes and P2P sync.
        if let Err(error) = remember_imgur_upload(&image.link, hash) {
            eprintln!("Could not save Imgur upload deletion record: {error}");
        }
    }
    Ok(image.link)
}

fn remember_imgur_upload(link: &str, hash: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let path = AppSettings::config_file()?.with_file_name("imgur-uploads.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let record = serde_json::json!({ "link": link, "deletehash": hash });
    writeln!(file, "{record}")?;
    Ok(())
}

#[tauri::command]
pub fn save_image_upload_settings(
    mut settings: ImageUploadSettings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    Provider::parse(&settings.provider)?;
    settings.local_default_applied = true;
    settings.imgur_client_id = settings.imgur_client_id.trim().into();
    if settings.provider == "imgur" { client_id(&settings.imgur_client_id)?; }
    let mut app = state.settings.write();
    let previous = std::mem::replace(&mut app.image_upload, settings);
    if let Err(error) = app.save() {
        app.image_upload = previous;
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn png() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\nsynthetic test image".to_vec()
    }

    #[test]
    fn imgur_has_its_own_limits_formats_and_public_client_id() {
        assert_eq!(client_id("").unwrap(), IMGUR_CLIENT_ID);
        assert_eq!(client_id("  custom123  ").unwrap(), "custom123");
        assert!(client_id("bad\r\nHeader: value").is_err());
        assert!(Provider::parse("drive").is_err());
        assert!(Provider::Imgur.image_type(&png()).is_ok());
        assert_eq!(
            Provider::Imgur.image_type(b"RIFF1234WEBP").unwrap_err(),
            "errors.imgurUnsupported"
        );
        let mut bytes = vec![0; MAX_IMGUR_BYTES + 1];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert!(Provider::Catbox.image_type(&bytes).is_ok());
        assert_eq!(
            Provider::Imgur.image_type(&bytes).unwrap_err(),
            "errors.imgurTooLarge"
        );
    }

    #[tokio::test]
    async fn imgur_sends_binary_multipart_with_public_id_and_retains_deletehash() {
        let body = r#"{"success":true,"data":{"link":"https://i.imgur.com/test.jpg","deletehash":"delete123"}}"#;
        let (endpoint, server) = mock_server(200, body).await;
        let result = upload_to(Provider::Imgur, &endpoint, png(), "myPublicId")
            .await
            .unwrap();
        assert_eq!(result.link, "https://i.imgur.com/test.jpg");
        assert_eq!(result.deletehash.as_deref(), Some("delete123"));
        let request = server.await.unwrap();
        let text = String::from_utf8_lossy(&request);
        assert!(text.contains("authorization: Client-ID myPublicId"));
        assert!(text.contains("name=\"image\"; filename=\"image.png\""));
        assert!(text.contains("name=\"type\"\r\n\r\nfile"));
        assert!(request.windows(png().len()).any(|window| window == png()));
        assert!(!text.contains("client_secret"));
        assert!(!text.contains("fileToUpload"));
    }

    #[tokio::test]
    async fn imgur_rejects_error_responses_invalid_links_and_quota_failures() {
        for (status, body, error) in [
            (401, "Denied", "errors.imgurRejected"),
            (429, "Slow down", "errors.imgurRateLimit"),
            (413, "Too large", "errors.imgurTooLarge"),
            (500, "Unavailable", "errors.imgurUpload"),
            (
                200,
                r#"{"success":false,"data":{"error":"Bad image"}}"#,
                "errors.imgurUpload",
            ),
            (
                200,
                r#"{"success":true,"data":{"link":"https://evil.com/a.png","deletehash":"abc"}}"#,
                "errors.imgurResponse",
            ),
            (
                200,
                r#"{"success":true,"data":{"link":"https://i.imgur.com/a.mp4","deletehash":"abc"}}"#,
                "errors.imgurResponse",
            ),
            (
                200,
                r#"{"success":true,"data":{"link":"https://i.imgur.com/a.png"}}"#,
                "errors.imgurResponse",
            ),
        ] {
            let (endpoint, server) = mock_server(status, body).await;
            assert_eq!(
                upload_to(Provider::Imgur, &endpoint, png(), "")
                    .await
                    .unwrap_err(),
                error
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "Uploads a synthetic 1x1 PNG to Imgur, then deletes it"]
    async fn live_imgur_upload_and_delete() {
        let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC").unwrap();
        let image = tokio::time::timeout(
            Duration::from_secs(45),
            upload_to(Provider::Imgur, IMGUR_ENDPOINT, bytes, ""),
        )
        .await
        .expect("Imgur live upload timed out")
        .expect("Imgur live upload failed");
        assert!(image.link.starts_with("https://i.imgur.com/"));
        let response = reqwest::Client::builder()
            .user_agent(concat!("LowNotes/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap()
            .delete(format!("{IMGUR_ENDPOINT}/{}", image.deletehash.unwrap()))
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Client-ID {IMGUR_CLIENT_ID}"),
            )
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "Could not delete synthetic Imgur fixture"
        );
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["success"], true);
    }

    #[tokio::test]
    #[ignore = "Uploads a synthetic 1x1 PNG to the real Catbox API"]
    async fn live_catbox_upload() {
        let bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC").unwrap();
        let result = tokio::time::timeout(Duration::from_secs(45), upload(ENDPOINT, bytes))
            .await
            .expect("Catbox live upload timed out");
        eprintln!("Live Catbox result: {result:?}");
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_browser_image_formats_and_enforces_catbox_limits() {
        assert_eq!(image_type(&png()).unwrap(), ("png", "image/png"));
        for bytes in [
            b"\xff\xd8\xff".as_slice(),
            b"GIF89a",
            b"GIF87a",
            b"RIFF1234WEBP",
            b"BM",
        ] {
            assert!(image_type(bytes).is_ok());
        }
        assert_eq!(
            image_type(b"not an image").unwrap_err(),
            "errors.imageUnsupported"
        );
        let mut gif = vec![0; MAX_GIF_BYTES + 1];
        gif[..6].copy_from_slice(b"GIF89a");
        assert_eq!(image_type(&gif).unwrap_err(), "errors.imageGifTooLarge");
        let mut huge = vec![0; MAX_IMAGE_BYTES + 1];
        huge[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert_eq!(image_type(&huge).unwrap_err(), "errors.imageTooLarge");
    }

    #[test]
    fn only_direct_https_catbox_image_links_are_accepted() {
        assert_eq!(
            validate_response(" https://files.catbox.moe/abc123.webp\n").unwrap(),
            "https://files.catbox.moe/abc123.webp"
        );
        for link in [
            "http://files.catbox.moe/a.png",
            "https://evil.com/a.png",
            "https://files.catbox.moe.evil.com/a.png",
            "https://user@files.catbox.moe/a.png",
            "https://files.catbox.moe/a.png?x=1",
            "https://files.catbox.moe/a.exe",
            "https://files.catbox.moe/path/a.png",
            "https://files.catbox.moe/a).png",
            "Upload rejected",
            "",
        ] {
            assert!(validate_response(link).is_err(), "{link}");
        }
    }

    async fn mock_server(status: u16, body: &str) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/user/api.php", listener.local_addr().unwrap());
        let body = body.to_owned();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let len = stream.read(&mut chunk).await.unwrap();
                if len == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..len]);
                if let Some(boundary) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..boundary]).to_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= boundary + 4 + size {
                        break;
                    }
                }
            }
            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (endpoint, task)
    }

    #[tokio::test]
    async fn sends_anonymous_multipart_upload_without_credentials() {
        let (endpoint, server) = mock_server(200, "https://files.catbox.moe/test.png\n").await;
        assert_eq!(
            upload(&endpoint, png()).await.unwrap(),
            "https://files.catbox.moe/test.png"
        );
        let request = server.await.unwrap();
        let text = String::from_utf8_lossy(&request);
        assert!(text.contains("POST /user/api.php"));
        assert!(text.contains("multipart/form-data; boundary="));
        assert!(text.contains("name=\"reqtype\"\r\n\r\nfileupload"));
        assert!(text.contains("name=\"fileToUpload\"; filename=\"image.png\""));
        assert!(text.contains("Content-Type: image/png"));
        assert!(text.contains(concat!("user-agent: LowNotes/", env!("CARGO_PKG_VERSION"))));
        assert!(request.windows(png().len()).any(|window| window == png()));
        assert!(!text.contains("userhash"));
        assert!(!text.to_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn translates_api_failures_and_rejects_success_status_with_error_body() {
        for (status, body, error) in [
            (403, "Denied", "errors.catboxRejected"),
            (429, "Slow down", "errors.catboxRateLimit"),
            (500, "Unavailable", "errors.catboxUpload"),
            (200, "Upload rejected", "errors.catboxResponse"),
        ] {
            let (endpoint, server) = mock_server(status, body).await;
            assert_eq!(upload(&endpoint, png()).await.unwrap_err(), error);
            server.await.unwrap();
        }
        let (endpoint, server) = mock_server(200, &"x".repeat(MAX_RESPONSE_BYTES + 1)).await;
        assert_eq!(
            upload(&endpoint, png()).await.unwrap_err(),
            "errors.catboxResponse"
        );
        server.await.unwrap();
    }
}
