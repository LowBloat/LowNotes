# Notes for coding agents

LowNotes is a Tauri 2 desktop app: Svelte 5 + TypeScript in `src/`, Rust in `src-tauri/src/`. Read [CONTRIBUTING.md](CONTRIBUTING.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before changing code.

## Verify every change

```bash
bun install --frozen-lockfile
bun run version:check
bun run check
bun run check:e2e
bun test
python -m unittest discover -s .github/scripts -p "test_*.py"
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --lib --tests -- -D warnings
cargo test --locked --lib --manifest-path src-tauri/Cargo.toml
bun run build
bun run test:e2e
```

Run `bunx playwright install chromium` once before the e2e suite.

## Rules

- **Locales:** user-facing strings live in `src/lib/i18n/locales/{en-US,pt-BR,es-ES}.ts`. Add every key to all three files; `tests/i18n.test.ts` enforces parity. Counts can use `key_one` variants.
- **Language:** code, comments, documentation, issues and pull requests are in English.
- **Versions:** never edit version numbers by hand. Use `bun run version:bump X.Y.Z`; it updates `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`.
- **Secrets:** never send keys, the vault identity or pairing tokens to the webview. Settings reach the interface through `AppSettings::for_frontend()`.
- **Commands:** IO-heavy Tauri commands run through `on_worker` in `commands.rs`, not on the main thread.
- **Storage:** writes go through `storage.rs` (atomic replace with validated backups) and structural changes through `structural::exclusive`; keep crash recovery working.
- **Sync protocol:** read [docs/SYNC-PROTOCOL.md](docs/SYNC-PROTOCOL.md) before touching `network.rs`. Older peers must keep syncing.
- **Dependencies:** keep dependency and version changes separate from fixes. `src-tauri/deny.toml` lists accepted advisories with a reason.
