# Security policy

LowNotes handles private notes, a per-vault P2P identity and optional AI provider keys. Thank you for helping keep them safe.

## Supported versions

Security fixes go into the latest release. Please update before reporting, and mention the exact version you tested (Settings shows it).

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Older releases | No |

## Reporting a vulnerability

**Do not open a public issue for a vulnerability.** Report it privately through GitHub:

1. Open the [Security tab](https://github.com/LowBloat/LowNotes/security) of the repository.
2. Choose **Report a vulnerability** and describe the problem, the affected version and operating system, and steps to reproduce.

Please do not include real notes, API keys or `LOWNOTES2_...` pairing codes. Use sample data.

We aim to acknowledge reports within 7 days and to agree on a disclosure date once a fix is ready. We credit reporters in the release notes unless you prefer otherwise.

## Scope

In scope: the desktop application in this repository, its P2P sync protocol, its update channel and the release workflow.

Out of scope: vulnerabilities in third-party AI providers, search engines or Iroh relay infrastructure themselves; attacks that require an already compromised operating system account; denial of service from a device you paired yourself.

See [docs/THREAT-MODEL.md](docs/THREAT-MODEL.md) for what LowNotes defends against and its known limits.
