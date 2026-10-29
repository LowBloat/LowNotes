# Connect AI apps over MCP

LowNotes can serve your notes to AI apps that speak the [Model Context Protocol](https://modelcontextprotocol.io/), such as Claude Desktop, Claude Code and Cursor. The app can search and read the vault open in LowNotes. It cannot create, change or delete notes.

The server runs inside LowNotes, so the app must be running. It can stay in the system tray (**Settings → General → Keep running in the system tray**).

## Turn it on

1. Open **Settings → MCP server** and turn on **Turn on the MCP server**. The first time, this also shares the vault that is open.
2. Under **Shared vaults**, check the vaults AI apps may read. An app only ever reads the vault that is open in LowNotes, and only while it is checked.
3. Under **Connect an AI app**, copy the configuration for your app.

### Claude Desktop

Add the `lownotes` entry to `mcpServers` in `claude_desktop_config.json` (**Settings → Developer → Edit Config** in Claude Desktop), then restart Claude Desktop:

```json
{
  "mcpServers": {
    "lownotes": { "command": "C:\\Program Files\\LowNotes\\lownotes-mcp.exe" }
  }
}
```

### Claude Code

```bash
claude mcp add --scope user lownotes -- "C:\Program Files\LowNotes\lownotes-mcp.exe"
```

### Cursor

Add the same `mcpServers` entry as Claude Desktop to `~/.cursor/mcp.json`.

Settings shows the right path for your installation. Typical locations:

| System | `lownotes-mcp` |
| --- | --- |
| Windows (installer) | `C:\Program Files\LowNotes\lownotes-mcp.exe` |
| Windows (portable) | next to `LowNotes.exe` |
| macOS | `/Applications/LowNotes.app/Contents/MacOS/lownotes-mcp` |
| Linux (.deb, .rpm, Arch) | `/usr/bin/lownotes-mcp` |

The AppImage mounts itself at a new path on every start, so a path copied from it stops working after a restart. Use the .deb, .rpm or Arch package to connect AI apps on Linux.

## What AI apps can do

| Tool | What it returns |
| --- | --- |
| `list_notes` | Paths, titles, dates and sizes of the notes, optionally inside one folder |
| `read_note` | The Markdown of one note (up to 512 KiB; longer notes come back truncated) |
| `search_notes` | The best matching excerpts for words or a phrase, with their note paths |
| `get_links` | The links between notes: wikilinks and links drawn in the map |
| `get_backlinks` | The notes that link to one note |

Each note is also available as a resource, `lownotes://note/{path}`. Paths inside `.lownotes/` and files that are not Markdown are refused.

Search uses the same local keyword search as the assistant: words, phrases and headings, without embeddings.

## Privacy and safety

- **What leaves LowNotes.** Everything an AI app reads goes to that app and to the AI provider it uses, under that provider's terms. Share only the vaults you are comfortable sending.
- **Notes from other devices.** A paired device can sync any text, including text written to steer an AI ("ignore your instructions and…"). LowNotes labels note content as data and tells clients not to follow instructions inside it, but it cannot control the AI app. Be careful when the same app can also run commands, send messages or browse.
- **Who can connect.** Only programs running as your user on this computer. `lownotes-mcp` presents an access token that LowNotes keeps in the system credential store; it never appears in the AI app's configuration or in LowNotes' interface.
- **Regenerate the token** (**Settings → MCP server → Regenerate token**) if you stop trusting an app that was connected. Every connected app is disconnected and has to reconnect.
- **Turn it off** at any time with the same switch; connected apps are disconnected at once.

The diagnostic log records which tool was called, how long it took and how large the answer was, never note content, search terms or the token.

## Troubleshooting

`lownotes-mcp` answers the AI app with an error that says what is wrong:

| Message | What to do |
| --- | --- |
| LowNotes is not open | Start LowNotes; it can stay in the tray. |
| The LowNotes MCP server is off | Turn it on in **Settings → MCP server**. |
| The vault open in LowNotes is not shared | Check the vault under **Shared vaults**, or open a shared vault. |
| LowNotes refused the token | The token was regenerated: restart or reconnect the AI app. |
| Could not read the MCP token | Unlock the system credential store (Keychain, Credential Manager or Secret Service). |

For more detail, run `lownotes-mcp` from a terminal with `LOWNOTES_LOG=info`; it logs to the terminal's error output.
