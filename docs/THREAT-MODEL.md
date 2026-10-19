# Threat model

This document describes what LowNotes protects, against whom, and the limits that remain. Report vulnerabilities as described in [SECURITY.md](../SECURITY.md).

## Assets

| Asset | Where it lives |
| --- | --- |
| Note content, images, history | Vault folder and `.lownotes/` on each device |
| Vault P2P identity (Iroh secret key) | OS credential store |
| Pairing token | OS credential store; embedded in shared pair codes |
| AI provider and search API keys | OS credential store |
| Assistant conversations | App data directory, per vault |

## Adversaries and mitigations

### Network attacker or relay operator
- All P2P traffic is QUIC encrypted and authenticated by endpoint keys; relays forward ciphertext.
- Relays and discovery services can observe that two endpoints communicate, and when.

### Unpaired endpoint that knows a device's address
- Only allowlisted endpoints can sync. Unknown endpoints get one small pairing request (16 KiB, 30 s) from a pool of 16 handshake slots; nothing larger is parsed before authorization.
- Pairing needs the token and the user's approval. The stored peer is the endpoint proven by the connection, not one claimed in the packet.
- Pair codes are single use: the token rotates after each accepted pairing and can be regenerated manually.

### Paired device (trusted)
- A paired device is fully trusted for that vault: it can add, change and delete notes, subject to conflict copies, history and trash for recovery.
- Content it sends is treated as untrusted when rendered: raw HTML is disabled in Markdown, Mermaid runs in strict mode, remote images do not load unless the user allows them, and a Content Security Policy blocks scripts that are not part of the app.

### Malicious note content (from a peer, an import or the assistant)
- No raw HTML; links in the preview never navigate the app; only `http(s)` and `mailto` open externally.
- Remote images are blocked by default to avoid IP leaks and tracking pixels (images from LowNotes' upload hosts are allowed).
- Exporting a note fetches remote images over HTTPS only and refuses loopback, private and link-local addresses, including after redirects and DNS resolution.

### Compromised interface (XSS despite the above)
- The webview never receives the vault identity, pairing token or API keys.
- Export writes only to a path the user picked in a native dialog; the webview gets a single-use token, not a path.
- The CSP limits script sources to the app itself and network connections to the IPC bridge.

### AI providers and search engines
- Remote providers receive the prompt and the note excerpts selected as context. Search sources receive the search query. Local providers (Ollama, LM Studio) keep this on the device.

### Update channel
- Updates are signed with the project's minisign key and verified before installation. Release assets have SHA-256 checksums, and the AUR recipe is published with real checksums.

## Known limits

- Installers are not yet code-signed for Windows or notarized for macOS (#9).
- `zip` used by the updater has open advisories without a compatible fix; it only unpacks signature-verified bundles (`src-tauri/deny.toml`).
- Path validation does not yet resolve symlinks and junctions inside the vault (roadmap B22).
- Anyone with access to the unlocked operating system account can read the vault files and, through the OS credential store, the stored secrets.
- Iroh discovery and relay services are third-party infrastructure.
