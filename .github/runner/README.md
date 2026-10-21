# Self-hosted Linux release runner

The `Linux x64` job of the [release workflow](../workflows/release.yml) can run on a self-hosted runner instead of GitHub's `ubuntu-22.04`. This folder holds the container that serves as that runner.

The repository variable `LINUX_RELEASE_RUNNER` chooses the runner:

| Value | Runner |
| --- | --- |
| unset | GitHub-hosted `ubuntu-22.04` |
| `["self-hosted","lownotes-linux-release"]` | this container |

Windows, macOS, the Arch package and every check keep running on GitHub-hosted runners.

## Why a container on Ubuntu 22.04

- The binaries link against the build machine's glibc. Ubuntu 22.04 keeps the same minimum glibc (2.35) as GitHub's runner, so a self-hosted build runs on the same distributions. The official `actions-runner` image moved to Ubuntu 24.04, so this image installs the runner on `ubuntu:22.04` itself.
- The container is not privileged and has no access to the host's Docker socket. A job can use `sudo` inside the container, as the workflow does for `apt-get`, but cannot reach the host's other containers. The Arch package needs Docker and therefore runs in a separate GitHub-hosted job.

## Security

This repository is public. Any pull request can change a workflow to target the `lownotes-linux-release` label, and the release job receives `TAURI_SIGNING_PRIVATE_KEY`.

- Only the release workflow uses the label. It runs on version tags and manual runs, which need write access.
- Set **Settings → Actions → General → Fork pull request workflows** to require approval for all external contributors, and read any workflow change before approving a run.
- The runner keeps its workspace between jobs. Recreate the container (`docker compose up -d --force-recreate`) if a job you did not expect ran on it.

## Set up

On a Linux host with Docker Compose, copy this folder (for example to `/opt/lownotes-runner`) and build the image:

```bash
docker compose build
```

Register the runner once. The registration token is valid for one hour and is read from stdin; an administrator of the repository can create it with the GitHub CLI:

```bash
gh api -X POST repos/LowBloat/LowNotes/actions/runners/registration-token --jq .token \
  | ssh HOST 'cd /opt/lownotes-runner && docker compose run --rm -T runner register'
```

The registration lands in the `state` volume. Start the runner:

```bash
docker compose up -d
docker compose logs -f runner
```

Then set the repository variable:

```bash
gh variable set LINUX_RELEASE_RUNNER --body '["self-hosted","lownotes-linux-release"]'
```

## Maintain

- **Runner updates:** the runner updates itself. To update the image's baseline, change `RUNNER_VERSION` and `RUNNER_SHA256` in the Dockerfile (the checksum is in the [runner release notes](https://github.com/actions/runner/releases)), rebuild and recreate the container. The registration survives in the `state` volume.
- **Resources:** `compose.yml` limits the container to 7 CPUs and 8 GB of memory.
- **Fall back to GitHub:** `gh variable delete LINUX_RELEASE_RUNNER`. While the variable points at the runner and the runner is offline, the job waits in the queue.
- **Test without publishing:** run the release workflow manually with `dry_run` enabled:

  ```bash
  gh workflow run release.yml -f tag=v0.4.0 -f dry_run=true
  ```

- **Remove:** `docker compose down -v` on the host, then remove the runner under **Settings → Actions → Runners**.
