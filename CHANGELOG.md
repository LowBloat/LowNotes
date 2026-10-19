# Changelog

All notable changes to LowNotes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

Releases up to 0.3.5 are described on the [GitHub releases page](https://github.com/LowBloat/LowNotes/releases).

## [Unreleased]

### Security
- Settings sent to the interface no longer contain the P2P identity, pairing tokens or provider/search API keys; key fields are write-only ([#2](https://github.com/LowBloat/LowNotes/issues/2)).
- A Content Security Policy is enforced, and remote images in notes and assistant messages load only after a click or when enabled in Settings ([#3](https://github.com/LowBloat/LowNotes/issues/3)).
- P2P: unpaired endpoints may only send a small pairing request, packets have size and time limits, the pairing request is bound to the connection identity and tokens are compared in constant time ([#6](https://github.com/LowBloat/LowNotes/issues/6)).
- Pair codes are single use and can be regenerated ([#7](https://github.com/LowBloat/LowNotes/issues/7)).
- Export destinations are chosen by a native dialog in the Rust process ([#11](https://github.com/LowBloat/LowNotes/issues/11)); remote images in exports must use HTTPS and public addresses ([#12](https://github.com/LowBloat/LowNotes/issues/12)).
- Dependency updates for rustls and lodash-es advisories; weekly dependency audit and Dependabot ([#10](https://github.com/LowBloat/LowNotes/issues/10)).

### Fixed
- Links in the preview no longer navigate the app away to a blank page ([#4](https://github.com/LowBloat/LowNotes/issues/4)).
- The update dialog shows the release notes again ([#8](https://github.com/LowBloat/LowNotes/issues/8)).

### Changed
- The AUR recipe is published with real checksums for every release ([#27](https://github.com/LowBloat/LowNotes/issues/27)).
- `bun run version:bump` and `bun run version:check` keep every version-bearing file in sync ([#22](https://github.com/LowBloat/LowNotes/issues/22)).
