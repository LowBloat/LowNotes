//! `lownotes-mcp`, the process MCP clients launch. It relays JSON-RPC lines
//! between the client's stdio and the running LowNotes (see `mcp.rs`) after
//! checking who serves the endpoint and presenting the token. It never opens
//! the vault itself.
//!
//! When LowNotes cannot be reached, the bridge answers the client's first
//! request with a JSON-RPC error that says why, then exits.
use std::{io, time::Duration};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::mcp::{self, Endpoint, HANDSHAKE};

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const FIRST_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long to wait while every pipe instance is busy.
#[cfg(windows)]
const BUSY_RETRIES: u32 = 40;
#[cfg(windows)]
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(50);
const MAX_REPLY_BYTES: usize = 64;

const NOT_ENABLED: &str =
    "The LowNotes MCP server is off. Turn it on in LowNotes: Settings > MCP server.";
const NOT_RUNNING: &str =
    "LowNotes is not open. Start LowNotes (it can stay in the tray) and reconnect.";
#[cfg(windows)]
const OTHER_USER: &str =
    "Refused to connect: the LowNotes MCP endpoint belongs to another user's process.";
const DENIED: &str =
    "LowNotes refused the token. If it was regenerated in LowNotes, reconnect the MCP client.";
const NO_CREDENTIALS: &str = "Could not read the MCP token from the system credential store.";
#[cfg(windows)]
const BUSY: &str = "LowNotes is serving too many MCP clients. Try again later.";
const FAILED: &str = "Could not reach LowNotes. See the bridge's log for details.";

/// Entry point of the `lownotes-mcp` binary; returns the exit code.
pub fn run() -> i32 {
    let filter = tracing_subscriber::EnvFilter::try_from_env("LOWNOTES_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .init();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!("could not start the MCP bridge: {error}");
            return 1;
        }
    };
    let code = runtime.block_on(async {
        let token = tokio::task::spawn_blocking(crate::credentials::mcp_token_for_bridge)
            .await
            .unwrap_or_else(|error| Err(error.into()));
        bridge(
            mcp::endpoint(),
            token,
            tokio::io::stdin(),
            tokio::io::stdout(),
        )
        .await
    });
    // Reading stdin blocks a thread that cannot be interrupted; do not wait for it.
    runtime.shutdown_background();
    code
}

/// Connects to LowNotes and relays until either side closes. Without a
/// connection, answers the first request with the reason instead.
pub(crate) async fn bridge<I, O>(
    endpoint: io::Result<Endpoint>,
    token: anyhow::Result<Option<String>>,
    input: I,
    output: O,
) -> i32
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    match open(endpoint, token).await {
        Ok(stream) => relay(stream, input, output).await,
        Err(reason) => {
            tracing::error!("{reason}");
            refuse(reason, input, output).await;
            1
        }
    }
}

async fn open(
    endpoint: io::Result<Endpoint>,
    token: anyhow::Result<Option<String>>,
) -> Result<Stream, &'static str> {
    let token = match token {
        Ok(Some(token)) => token,
        Ok(None) => return Err(NOT_ENABLED),
        Err(error) => {
            tracing::warn!("could not read the MCP token: {error:#}");
            return Err(NO_CREDENTIALS);
        }
    };
    let endpoint = endpoint.map_err(|error| {
        tracing::warn!("could not locate the MCP endpoint: {error}");
        FAILED
    })?;
    let mut stream = connect(&endpoint).await?;
    let mut hello = HANDSHAKE.to_vec();
    hello.extend_from_slice(token.as_bytes());
    hello.push(b'\n');
    stream.write_all(&hello).await.map_err(|error| {
        tracing::warn!("MCP handshake failed: {error}");
        NOT_RUNNING
    })?;
    match tokio::time::timeout(REPLY_TIMEOUT, read_reply(&mut stream)).await {
        Ok(Ok(reply)) if reply == "OK" => Ok(stream),
        Ok(Ok(reply)) if reply == "DENIED" => Err(DENIED),
        _ => Err(NOT_RUNNING),
    }
}

/// The server's one-line answer, read byte by byte so no MCP data is consumed.
async fn read_reply(stream: &mut Stream) -> io::Result<String> {
    let mut line = Vec::new();
    loop {
        let byte = stream.read_u8().await?;
        if byte == b'\n' || line.len() == MAX_REPLY_BYTES {
            break;
        }
        line.push(byte);
    }
    Ok(String::from_utf8_lossy(&line).trim().to_owned())
}

#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;

#[cfg(windows)]
async fn connect(endpoint: &Endpoint) -> Result<Stream, &'static str> {
    use std::os::windows::io::AsRawHandle;
    use tokio::net::windows::named_pipe::ClientOptions;
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

    for _ in 0..BUSY_RETRIES {
        match ClientOptions::new().open(endpoint) {
            Ok(client) => {
                let owner = mcp::pipe_security::server_user_sid(client.as_raw_handle());
                let me = mcp::pipe_security::current_user_sid();
                return match (owner, me) {
                    (Ok(owner), Ok(me)) if owner == me => Ok(client),
                    (Ok(_), Ok(_)) => Err(OTHER_USER),
                    (Err(error), _) | (_, Err(error)) => {
                        tracing::warn!("could not check the MCP endpoint owner: {error}");
                        Err(OTHER_USER)
                    }
                };
            }
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(BUSY_RETRY_DELAY).await;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(NOT_RUNNING),
            Err(error) => {
                tracing::warn!("could not open the MCP endpoint: {error}");
                return Err(NOT_RUNNING);
            }
        }
    }
    Err(BUSY)
}

/// The socket sits in a folder only this user can enter, so whoever serves
/// it runs as this user.
#[cfg(unix)]
async fn connect(endpoint: &Endpoint) -> Result<Stream, &'static str> {
    tokio::net::UnixStream::connect(endpoint)
        .await
        .map_err(|error| {
            if !matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) {
                tracing::warn!("could not open the MCP endpoint: {error}");
            }
            NOT_RUNNING
        })
}

async fn relay<I, O>(stream: Stream, mut input: I, mut output: O) -> i32
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let (mut from_app, mut to_app) = tokio::io::split(stream);
    let upstream = async {
        let copied = tokio::io::copy(&mut input, &mut to_app).await;
        let _ = to_app.shutdown().await;
        copied
    };
    let downstream = async {
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = from_app.read(&mut buffer).await?;
            if read == 0 {
                return Ok::<_, io::Error>(());
            }
            output.write_all(&buffer[..read]).await?;
            output.flush().await?;
        }
    };
    tokio::select! {
        copied = upstream => match copied {
            Ok(_) => 0,
            Err(error) => {
                tracing::error!("reading from the MCP client failed: {error}");
                1
            }
        },
        forwarded = downstream => {
            match forwarded {
                Ok(()) => tracing::error!("LowNotes closed the MCP connection"),
                Err(error) => tracing::error!("relaying to the MCP client failed: {error}"),
            }
            1
        }
    }
}

/// Answers the first request the client sends with `reason`.
async fn refuse<I, O>(reason: &str, input: I, mut output: O)
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(input).lines();
    let first_request = async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(id) = request_id(&line) {
                return Some(id);
            }
        }
        None
    };
    let Ok(Some(id)) = tokio::time::timeout(FIRST_REQUEST_TIMEOUT, first_request).await else {
        return;
    };
    let _ = output
        .write_all(error_response(id, reason).as_bytes())
        .await;
    let _ = output.flush().await;
}

/// The id of a JSON-RPC request; notifications have none.
fn request_id(line: &str) -> Option<Value> {
    let message: Value = serde_json::from_str(line).ok()?;
    message.get("method")?;
    message.get("id").filter(|id| !id.is_null()).cloned()
}

fn error_response(id: Value, reason: &str) -> String {
    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32000, "message": reason },
    });
    format!("{response}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_requests_but_not_notifications() {
        assert_eq!(
            request_id(r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#),
            Some(json!(7))
        );
        assert_eq!(
            request_id(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#),
            Some(json!("a"))
        );
        assert_eq!(
            request_id(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
        );
        assert_eq!(request_id("not json"), None);
        let reply: Value = serde_json::from_str(&error_response(json!(7), NOT_RUNNING)).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["message"], NOT_RUNNING);
    }

    async fn refused_with(
        endpoint: io::Result<Endpoint>,
        token: anyhow::Result<Option<String>>,
    ) -> (i32, Value) {
        let (mut client_in, bridge_in) = tokio::io::duplex(4096);
        let (bridge_out, client_out) = tokio::io::duplex(4096);
        client_in
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/x\"}\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n")
            .await
            .unwrap();
        let code = bridge(endpoint, token, bridge_in, bridge_out).await;
        let line = BufReader::new(client_out)
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        (code, serde_json::from_str(&line).unwrap())
    }

    #[tokio::test]
    async fn tells_the_client_why_lownotes_cannot_be_reached() {
        use rand::Rng;
        let suffix = rand::rng().random::<u64>();
        #[cfg(unix)]
        let missing = std::env::temp_dir().join(format!("lownotes-mcp-missing-{suffix:016x}"));
        #[cfg(windows)]
        let missing = format!(r"\\.\pipe\lownotes-mcp-missing-{suffix:016x}");
        let (code, reply) = refused_with(Ok(missing), Ok(Some("token".into()))).await;
        assert_eq!(code, 1);
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["error"]["message"], NOT_RUNNING);

        let (_, reply) = refused_with(Err(io::Error::other("no dir")), Ok(None)).await;
        assert_eq!(reply["error"]["message"], NOT_ENABLED);
    }
}
