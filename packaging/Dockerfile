# Builds the Debian package in a clean trixie environment.
#   docker build --platform linux/amd64 -f packaging/Dockerfile --output dist .
FROM debian:trixie AS build

ARG RUST_VERSION=1.92.0
ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:$PATH

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential debhelper devscripts help2man lintian \
      ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

RUN curl -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}"

WORKDIR /build/agent-memory-mcp

# Fetch crates in their own layer so source edits don't re-download them.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && touch src/lib.rs && cargo fetch --locked && rm -rf src

COPY . .
RUN dpkg-buildpackage --build=binary --no-sign \
 && lintian --info --display-info --fail-on error ../*.changes \
 && mkdir /out && cp ../*.deb ../*.buildinfo ../*.changes /out/

FROM scratch
COPY --from=build /out/ /
