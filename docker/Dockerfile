FROM rust:1.99-bookworm AS server
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends \
  pkg-config libssl-dev \
  && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY backend ./backend
COPY assets ./assets
ARG CARGO_TARGET_CACHE=ocr-cargo-target
RUN --mount=type=cache,id=ocr-cargo-registry,target=/usr/local/cargo/registry,sharing=locked --mount=type=cache,id=${CARGO_TARGET_CACHE},target=/app/target,sharing=locked CARGO_BUILD_JOBS=2 cargo build --locked --release --bin ocr-server \
  && cp /app/target/release/ocr-server /usr/local/bin/

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl git bubblewrap \
  && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
  && chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg \
  && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list \
  && apt-get update \
  && apt-get install -y --no-install-recommends gh \
  && rm -rf /var/lib/apt/lists/*
ARG TARGETARCH
ARG CLAUDE_CLI_VERSION=2.1.292
ARG CLAUDE_CLI_SHA256_AMD64=a967e7b1d8b4e47ee421d5433027880347952b0c0857abf880e2c942a4ec93b3
ARG CLAUDE_CLI_SHA256_ARM64=24caa9e6ff13bf227049a2626f1c816fc895023050f0ec3b12dbf14d897367e0
RUN case "$TARGETARCH" in \
      amd64) platform=linux-x64; checksum="$CLAUDE_CLI_SHA256_AMD64" ;; \
      arm64) platform=linux-arm64; checksum="$CLAUDE_CLI_SHA256_ARM64" ;; \
      *) echo "Unsupported Claude CLI architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
  && mkdir -p /opt/claude/bin \
  && curl -fsSL --retry 3 "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases/${CLAUDE_CLI_VERSION}/${platform}/claude" -o /opt/claude/bin/claude \
  && printf '%s  /opt/claude/bin/claude\n' "$checksum" | sha256sum -c - \
  && chmod 0755 /opt/claude/bin/claude \
  && ln -s /opt/claude/bin/claude /usr/local/bin/claude \
  && test "$(DISABLE_AUTOUPDATER=1 /opt/claude/bin/claude --version | cut -d ' ' -f1)" = "$CLAUDE_CLI_VERSION"
LABEL ai.claude.cli.version="${CLAUDE_CLI_VERSION}"
ARG CODEX_CLI_VERSION=0.163.0
ARG CODEX_CLI_SHA256_AMD64=beb339e8ecd9ba057a6bb2dc81ad8ca474812edfbf0fc5056ca818797648dde4
ARG CODEX_CLI_SHA256_ARM64=a7261e65f4a96d8f4ca65a069c6cf2dc2c33defc341e48078acefda9461fffe4
RUN case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-musl; checksum="$CODEX_CLI_SHA256_AMD64" ;; \
      arm64) target=aarch64-unknown-linux-musl; checksum="$CODEX_CLI_SHA256_ARM64" ;; \
      *) echo "Unsupported Codex CLI architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
  && mkdir -p /opt/codex/bin \
  && curl -fsSL --retry 3 "https://github.com/openai/codex/releases/download/rust-v${CODEX_CLI_VERSION}/codex-${target}.tar.gz" -o /tmp/codex.tar.gz \
  && printf '%s  /tmp/codex.tar.gz\n' "$checksum" | sha256sum -c - \
  && tar -xzf /tmp/codex.tar.gz -C /opt/codex/bin \
  && mv "/opt/codex/bin/codex-${target}" /opt/codex/bin/codex \
  && rm /tmp/codex.tar.gz \
  && chmod 0755 /opt/codex/bin/codex \
  && ln -s /opt/codex/bin/codex /usr/local/bin/codex \
  && test "$(/opt/codex/bin/codex --version | cut -d ' ' -f2)" = "$CODEX_CLI_VERSION"
LABEL ai.codex.cli.version="${CODEX_CLI_VERSION}"
ARG NODE_VERSION=22.23.3
ARG NODE_SHA256_AMD64=df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de
ARG NODE_SHA256_ARM64=a44aeb94849a299b22df10b9e622ec2f605c2183501bc40590705131de7c740f
RUN case "$TARGETARCH" in \
      amd64) platform=linux-x64; checksum="$NODE_SHA256_AMD64" ;; \
      arm64) platform=linux-arm64; checksum="$NODE_SHA256_ARM64" ;; \
      *) echo "Unsupported Node.js architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
  && apt-get update && apt-get install -y --no-install-recommends xz-utils && rm -rf /var/lib/apt/lists/* \
  && curl -fsSL --retry 3 "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-${platform}.tar.xz" -o /tmp/node.tar.xz \
  && printf '%s  /tmp/node.tar.xz\n' "$checksum" | sha256sum -c - \
  && mkdir -p /opt/node \
  && tar -xJf /tmp/node.tar.xz -C /opt/node --strip-components=1 "node-v${NODE_VERSION}-${platform}/bin/node" \
  && rm /tmp/node.tar.xz \
  && test "$(/opt/node/bin/node --version)" = "v${NODE_VERSION}"
LABEL org.nodejs.version="${NODE_VERSION}"
ARG GEMINI_CLI_VERSION=0.63.0
ARG GEMINI_CLI_SHA256=97a6edfc10645463b517f0518d46a8c72efbdc12558a9a948607f726284a0420
RUN curl -fsSL --retry 3 "https://registry.npmjs.org/@google/gemini-cli/-/gemini-cli-${GEMINI_CLI_VERSION}.tgz" -o /tmp/gemini.tgz \
  && printf '%s  /tmp/gemini.tgz\n' "$GEMINI_CLI_SHA256" | sha256sum -c - \
  && mkdir -p /opt/gemini \
  && tar -xzf /tmp/gemini.tgz -C /opt/gemini --strip-components=1 \
  && rm /tmp/gemini.tgz \
  && printf '#!/bin/sh\nGEMINI_CLI_NO_RELAUNCH=true exec /opt/node/bin/node /opt/gemini/bundle/gemini.js "$@"\n' > /usr/local/bin/gemini \
  && chmod 0755 /usr/local/bin/gemini \
  && test "$(HOME=/tmp gemini --version)" = "$GEMINI_CLI_VERSION" \
  && rm -rf /tmp/.gemini
LABEL ai.gemini.cli.version="${GEMINI_CLI_VERSION}"
ARG OPENCODE_CLI_VERSION=1.18.35
ARG OPENCODE_CLI_SHA256_AMD64=c8f888b451f5494a18f858fffb0e0b68f4e4baa9c241761c5f206884f0fa640d
ARG OPENCODE_CLI_SHA256_ARM64=f7f2ba59ee8aa94d388f9696575a32d20e71c2ee48def9f80fc693a60fec6c72
RUN case "$TARGETARCH" in \
      amd64) platform=linux-x64; checksum="$OPENCODE_CLI_SHA256_AMD64" ;; \
      arm64) platform=linux-arm64; checksum="$OPENCODE_CLI_SHA256_ARM64" ;; \
      *) echo "Unsupported OpenCode CLI architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
  && curl -fsSL --retry 3 "https://github.com/anomalyco/opencode/releases/download/v${OPENCODE_CLI_VERSION}/opencode-${platform}.tar.gz" -o /tmp/opencode.tar.gz \
  && printf '%s  /tmp/opencode.tar.gz\n' "$checksum" | sha256sum -c - \
  && mkdir -p /opt/opencode/bin \
  && tar -xzf /tmp/opencode.tar.gz -C /opt/opencode/bin \
  && rm /tmp/opencode.tar.gz \
  && chown -R root:root /opt/opencode \
  && chmod 0755 /opt/opencode/bin/opencode \
  && ln -s /opt/opencode/bin/opencode /usr/local/bin/opencode \
  && test "$(HOME=/tmp/opencode-home XDG_DATA_HOME=/tmp/opencode-home XDG_CONFIG_HOME=/tmp/opencode-home XDG_CACHE_HOME=/tmp/opencode-home XDG_STATE_HOME=/tmp/opencode-home /opt/opencode/bin/opencode --version)" = "$OPENCODE_CLI_VERSION" \
  && rm -rf /tmp/opencode-home
LABEL ai.opencode.cli.version="${OPENCODE_CLI_VERSION}"
ENV DISABLE_AUTOUPDATER=1
WORKDIR /app
COPY --from=server /app/assets ./assets
COPY --from=server /usr/local/bin/ocr-server /usr/local/bin/ocr-server
ENV OCR_BIND=0.0.0.0:8787 \
  OCR_DATA_DIR=/data \
  OCR_DB_PATH=/data/opencodereview.sqlite \
  OCR_REPOS_DIR=/data/repos
VOLUME ["/data"]
EXPOSE 8787
CMD ["ocr-server"]
