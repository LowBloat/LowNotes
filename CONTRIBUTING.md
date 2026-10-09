# Contributing to LowNotes

LowNotes is an open-source Markdown note-taking app built with Svelte, TypeScript, Tauri and Rust. Contributions can include bug fixes, documentation, app interface translations, accessibility improvements and tests.

Keep repository documentation, issues and pull requests in English so contributors around the world can participate. App locale files retain their target language.

## Before you start

- Search [existing issues](https://github.com/LowBloat/LowNotes/issues) and [pull requests](https://github.com/LowBloat/LowNotes/pulls) for related work.
- For a larger feature or a sync/storage change, open an issue describing the problem and proposed approach before implementing it.
- Keep each pull request focused on one problem. Include a concrete before/after example when it helps explain the change.

## Report a bug or suggest a feature

Use the [issue forms](https://github.com/LowBloat/LowNotes/issues/new/choose). Bug reports should include the LowNotes version, operating system, installation format, steps to reproduce, and expected and actual behavior. For sync issues, include both device versions and whether each device was online.

Use a small synthetic vault for examples. Remove personal notes, provider keys, pairing codes and other sensitive data from screenshots and logs.

## Set up development

Install [Bun](https://bun.sh/), the stable [Rust toolchain](https://rustup.rs/) and the system dependencies for Tauri on your operating system. The [release workflow](.github/workflows/release.yml) lists the Linux desktop libraries used by the project.

Fork the repository, then clone your fork and create a branch:

```bash
git clone https://github.com/YOUR-USERNAME/LowNotes.git
cd LowNotes
git switch -c fix/short-description
bun install --frozen-lockfile
bun run tauri dev
```

Use a disposable vault when changing persistence, deletion, recovery or synchronization behavior.

## Verify your changes

Run the checks relevant to your change. For app code, the verification commands are:

```bash
bun run check
bun run check:e2e
bun test
python .github/scripts/test_update_manifest.py
cargo test --locked --lib --manifest-path src-tauri/Cargo.toml
bun run build
bunx playwright install chromium
bun run test:e2e
```

For documentation-only changes, verify examples, relative links and image paths; app tests are unnecessary unless behavior also changes. CI runs the app checks on Windows, Linux and macOS.

For a bug fix, add a regression test when it demonstrates the failure. Browser tests cover the frontend through an isolated native adapter; Rust tests cover native persistence and synchronization. Changes involving offline edits, moves or deletions should demonstrate that existing content remains recoverable.

## Open a pull request

Explain the problem, the resulting behavior and how you verified it. Link a related issue if one exists. Include screenshots for visible UI changes and disclose any checks you could not run.

Keep dependency updates and version changes separate unless they are needed for the fix. Follow the surrounding code style and keep user-facing strings in the app's locale dictionaries.

LowNotes uses the [AGPL-3.0-only license](LICENSE); contributions must be compatible with the project's license. Be respectful and give actionable, specific feedback during review.
