# Self-hosted runner for the Linux release build. Ubuntu 22.04 matches the
# GitHub-hosted ubuntu-22.04 image, so the binaries keep the same minimum glibc
# whichever runner builds them. See README.md in this folder.
FROM ubuntu:22.04

ARG RUNNER_VERSION=2.338.0
ARG RUNNER_SHA256=af4b794c1bc41d73d40535e3fe092a39f9679cd8d965954c2aca25a05ca41d32

ENV DEBIAN_FRONTEND=noninteractive

# Build tools plus the desktop libraries the release workflow installs, so its
# apt-get step finds them already present. The AppImage bundler needs xdg-open,
# which GitHub's runner image has preinstalled.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl file git jq pkg-config python3 sudo unzip xz-utils zstd \
      libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libappindicator3-dev \
      librsvg2-dev libssl-dev libdbus-1-dev patchelf xdg-utils \
 && rm -rf /var/lib/apt/lists/*

# The workflow calls sudo apt-get, as on GitHub-hosted runners.
RUN useradd --create-home --uid 1001 runner \
 && echo 'runner ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/runner \
 && chmod 0440 /etc/sudoers.d/runner

WORKDIR /home/runner
RUN curl -fsSLo runner.tar.gz "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
 && echo "${RUNNER_SHA256}  runner.tar.gz" | sha256sum -c - \
 && tar -xzf runner.tar.gz \
 && rm runner.tar.gz \
 && ./bin/installdependencies.sh \
 && rm -rf /var/lib/apt/lists/* \
 && mkdir state \
 && chown -R runner:runner /home/runner

COPY --chmod=0755 entrypoint.sh /usr/local/bin/runner-entrypoint

USER runner
# Containers have no FUSE; the AppImage tools must unpack themselves instead.
ENV APPIMAGE_EXTRACT_AND_RUN=1
ENTRYPOINT ["/usr/local/bin/runner-entrypoint"]
CMD ["run"]
