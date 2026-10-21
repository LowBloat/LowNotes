# Changelog

All notable changes to LowNotes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

Releases up to 0.3.5 are described on the [GitHub releases page](https://github.com/LowBloat/LowNotes/releases).

## [Unreleased]

## [0.4.1] - 2026-10-21

### Added
- "Open with LowNotes" in the file manager for folders and Markdown files on Windows and Linux, off by default (Settings → System integration). Launching LowNotes with a folder or note opens it, and asks before turning a new folder into a vault ([#75](https://github.com/LowBloat/LowNotes/issues/75)).

### Fixed
- The update dialog renders release notes as formatted Markdown instead of raw text; their links open in the browser ([#73](https://github.com/LowBloat/LowNotes/issues/73)).

### Security
- markdown-it 15.0.3 fixes quadratic-time rendering of crafted link reference definitions and inline HTML, which synced notes could trigger; rustls-webpki and the bundled root certificates no longer trust distrusted intermediates ([#79](https://github.com/LowBloat/LowNotes/pull/79)).

### Development
- The Linux release build can run on a self-hosted runner chosen by the `LINUX_RELEASE_RUNNER` repository variable; `.github/runner` describes the container. The Arch package has its own job, and manual release runs can skip publishing with `dry_run` ([#76](https://github.com/LowBloat/LowNotes/issues/76)).

## [0.4.0] - 2026-10-19

### Added
- Remove a vault from the app's list; its folder and notes stay on disk. Missing folders are flagged ([#35](https://github.com/LowBloat/LowNotes/issues/35)).
- "New code" in Manage Connections invalidates every pair code shared so far ([#7](https://github.com/LowBloat/LowNotes/issues/7)).
- "Load remote images" setting, off by default ([#3](https://github.com/LowBloat/LowNotes/issues/3)).
- Diagnostic log in the app data folder, with "Open log folder" in Settings. It never contains note content, keys or pairing codes ([#23](https://github.com/LowBloat/LowNotes/issues/23)).
- Releases include `SHA256SUMS`, an SPDX SBOM and build provenance attestations ([#34](https://github.com/LowBloat/LowNotes/issues/34)).

### Changed
- Web search services are opt-in; only DuckDuckGo is on by default. Existing settings keep DuckDuckGo and any service configured with a key ([#25](https://github.com/LowBloat/LowNotes/issues/25)).
- In-app dialogs replace the native alert, confirm and prompt windows, which also fixes renaming notes and chats on macOS ([#20](https://github.com/LowBloat/LowNotes/issues/20)).
- The tray menu, the vault folder picker and the palette preview follow the interface language; counts use proper singular and plural forms ([#30](https://github.com/LowBloat/LowNotes/issues/30)).
- Modals take focus, keep Tab inside and return focus when closed; sidebar rows are plain buttons; the vault menu closes on Escape or an outside click ([#24](https://github.com/LowBloat/LowNotes/issues/24)).
- Every conflict copy stays listed until it is opened or dismissed ([#19](https://github.com/LowBloat/LowNotes/issues/19)).
- Offline peers are retried with exponential backoff and report one error instead of one per minute ([#17](https://github.com/LowBloat/LowNotes/issues/17)).
- Pair codes are single use ([#7](https://github.com/LowBloat/LowNotes/issues/7)).
- The AUR recipe is published with real checksums for every release ([#27](https://github.com/LowBloat/LowNotes/issues/27)).

### Performance
- Disk, CRDT and sync work no longer runs on the interface thread, and structural locks are per vault ([#5](https://github.com/LowBloat/LowNotes/issues/5)).
- Typing sends merged updates every 150 ms instead of one native call and durable save per keystroke; the preview follows the text with a short delay and notes are read once when opened ([#15](https://github.com/LowBloat/LowNotes/issues/15), [#16](https://github.com/LowBloat/LowNotes/issues/16)).
- Periodic sync reuses file hashes, skips unchanged CRDT documents, caches titles and slows down for idle peers ([#13](https://github.com/LowBloat/LowNotes/issues/13)).
- The resolved structural catalog is cached ([#18](https://github.com/LowBloat/LowNotes/issues/18)).
- Live collaboration reuses one connection per peer and coalesces queued updates ([#14](https://github.com/LowBloat/LowNotes/issues/14)).

### Fixed
- Links in the preview no longer navigate the app away to a blank page ([#4](https://github.com/LowBloat/LowNotes/issues/4)).
- The update dialog shows the release notes again ([#8](https://github.com/LowBloat/LowNotes/issues/8)).
- A vault identity that cannot be saved no longer goes online and gets replaced on the next start ([#23](https://github.com/LowBloat/LowNotes/issues/23)).
- Restarting the network closes the previous connection endpoint first ([#26](https://github.com/LowBloat/LowNotes/issues/26)).
- File operations on Windows tolerate brief locks from antivirus and indexers ([#50](https://github.com/LowBloat/LowNotes/issues/50)).
- Replaced secrets no longer leave orphaned entries in the OS credential store ([#31](https://github.com/LowBloat/LowNotes/issues/31)).

### Security
- Settings sent to the interface no longer contain the P2P identity, pairing tokens or provider/search API keys; key fields are write-only ([#2](https://github.com/LowBloat/LowNotes/issues/2)).
- A Content Security Policy is enforced, and remote images in notes and assistant messages load only after a click or when enabled in Settings ([#3](https://github.com/LowBloat/LowNotes/issues/3)).
- P2P: unpaired endpoints may only send a small pairing request, packets have size and time limits, the pairing request is bound to the connection identity and tokens are compared in constant time ([#6](https://github.com/LowBloat/LowNotes/issues/6)).
- Export destinations are chosen by a native dialog in the Rust process ([#11](https://github.com/LowBloat/LowNotes/issues/11)); remote images in exports must use HTTPS and public addresses ([#12](https://github.com/LowBloat/LowNotes/issues/12)).
- Dependency updates for rustls and lodash-es advisories; weekly dependency audit and Dependabot ([#10](https://github.com/LowBloat/LowNotes/issues/10)).
- The opener plugin no longer accepts custom file flags from the interface (tauri-plugin-opener 2.7).

### Development
- `bun run version:bump` and `bun run version:check` keep every version-bearing file in sync ([#22](https://github.com/LowBloat/LowNotes/issues/22)).
- CI caches dependencies, runs the interface suite once, checks `rustfmt` and `clippy`, smoke-tests bundling and is reused by the release workflow ([#21](https://github.com/LowBloat/LowNotes/issues/21)).
- Unused dependencies, template leftovers and dead Rust code were removed ([#28](https://github.com/LowBloat/LowNotes/issues/28), [#29](https://github.com/LowBloat/LowNotes/issues/29), [#31](https://github.com/LowBloat/LowNotes/issues/31)).
- Dependency updates: Tauri plugins, iroh 1.3, rusqlite 0.40, base64 0.23, html-escape 0.3 and GitHub Actions.
- Community, security and design documentation ([#32](https://github.com/LowBloat/LowNotes/issues/32), [#33](https://github.com/LowBloat/LowNotes/issues/33)).

[Unreleased]: https://github.com/LowBloat/LowNotes/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/LowBloat/LowNotes/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/LowBloat/LowNotes/compare/v0.3.5...v0.4.0
