# Roadmap

Open proposals from the October 2026 backlog review, in English. The original Portuguese backlog and the P1 implementation log are archived in [history/](history/). Items B01–B05, B07 and B08 (durable deletes, moves with identity, atomic saves, trash and history, mergeable map links, PR checks, OS credential store) are delivered.

Sizes: S small, M medium, L large. Where a GitHub issue covers part of an item, it is linked.

## Reliability

| Item | Proposal | Size |
| --- | --- | --- |
| B06 | Reproducible RAM, CPU and latency report per platform (main process and WebView), with 100 / 1,000 / 10,000-note vaults. No numeric target before measuring. | M |

## Daily use and scale

| Item | Proposal | Size |
| --- | --- | --- |
| B09 | Manual and scheduled backup and validated restore of the whole vault (Markdown, `.lownotes`, images, collaborative history), optionally including assistant conversations without credentials. | M |
| B10 | Shared incremental local index for content search and assistant context; search note bodies with highlighted excerpts. Related: [#13](https://github.com/LowBloat/LowNotes/issues/13), [#18](https://github.com/LowBloat/LowNotes/issues/18). | M/L |
| B11 | Better retrieval: a query set with expected results, accent-insensitive matching, Markdown-aware chunks, context limits per model, choosing and reviewing the excerpts sent. | M |
| B12 | Watch the filesystem so edits from other editors update the sidebar, open note, index and map without overwriting local changes. | M |
| B13 | Stream assistant responses and cancel them in the backend; never apply partial proposals. | M |
| B14 | Review assistant link proposals (and the text changes they imply) before applying, with undo for the batch. | S/M |
| B15 | Fluid map for large vaults: layout off the interface thread, fewer recomputations, filters by folder, origin and distance. | M/L |
| B16 | Syntax highlighting inside fenced code blocks, parsers loaded on demand. | M |
| B17 | Backlinks panel, `[[` autocompletion and visible broken or ambiguous references. | M |
| B18 | Vault-wide task view that edits the original Markdown, participates in undo and syncs. | M |
| B19 | Export Markdown with portable image files and rewritten relative paths, for one note or the whole vault. | M |
| B20 | Image storage usage and safe cleanup that respects history, trash and offline peers. | M |
| B21 | Consistent in-app dialogs, command palette, favorites, recent notes and discoverable shortcuts. Related: [#20](https://github.com/LowBloat/LowNotes/issues/20), [#24](https://github.com/LowBloat/LowNotes/issues/24). | S/M |
| B22 | Resolve symlinks and junctions against the vault root; review native command permissions. The CSP part is done ([#3](https://github.com/LowBloat/LowNotes/issues/3)). | M |
| B28 | Keep documentation in step with releases; troubleshooting guide for pairing, sync and updates that does not require sharing notes or keys. | S/M |

## Expansion

| Item | Proposal | Size |
| --- | --- | --- |
| B23 | Optional sync adapters for external storage (S3/WebDAV first), preserving identities, deletes and conflicts. | L |
| B24 | Maintained Linux channels (AUR, Flathub, DEB/RPM repositories) with automated metadata. Checksums: [#27](https://github.com/LowBloat/LowNotes/issues/27). | M/L |
| B25 | Mobile client for reading, editing, images and tasks with sync. | L |
| B26 | Optional note templates (daily, meeting, study, project). | S/M |
| B27 | Generate and check brand assets from the canonical logo before each release. | S |

## Suggested order

1. Backup and portable Markdown export (B09, B19).
2. Content search, index and file watching (B10, B12).
3. Review of assistant changes and vault boundaries (B14, B22), then retrieval quality and streaming (B11, B13).
4. Map, code highlighting, organization, tasks and image space (B15–B21).
5. External storage and new platforms (B23–B25). Templates, brand automation and documentation (B26–B28) can land as small deliveries at any time.
