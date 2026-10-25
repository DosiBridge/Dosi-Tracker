# Linux agent CI image: the same container runs locally (Docker Desktop on any
# host) and in .github/workflows/linux.yml, so a green local run means a green CI
# run. Built on ubuntu:22.04 so release binaries link against glibc 2.35, the
# oldest glibc the packages support.
#
# Build:  docker build -t dosi-linux-ci:22.04 -f clients/linux/ci/Dockerfile clients/linux/ci
# Run:    docker run --rm --user builder -v "$REPO:/repo" -v dosi-linux-target:/target \
#           -v dosi-linux-cargo:/usr/local/cargo/registry -e CARGO_TARGET_DIR=/target \
#           -w /repo/clients/linux dosi-linux-ci:22.04 bash ci/check.sh all
FROM ubuntu:22.04

ARG RUST_TOOLCHAIN=1.97.1
ARG BUILDER_UID=1000

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Build tools, the private buses and Xvfb harness, GUI runtime libraries (the
# GUI dlopens them, nothing links them), and the packaging/validation tools.
# No -dev packages: the build is pure Rust plus a C compiler.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      appstream \
      build-essential \
      ca-certificates \
      curl \
      dbus \
      dbus-x11 \
      desktop-file-utils \
      dpkg-dev \
      file \
      git \
      libegl1 \
      libgl1 \
      libgl1-mesa-dri \
      libx11-xcb1 \
      libxcursor1 \
      libxi6 \
      libxkbcommon-x11-0 \
      libxrandr2 \
      openbox \
      pkg-config \
      procps \
      python3 \
      rpm \
      shellcheck \
      systemd \
      x11-utils \
      xauth \
      xdotool \
      xterm \
      xvfb \
 && rm -rf /var/lib/apt/lists/*

# Pinned toolchain, shared read-only by every user of the image.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh \
 && sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal \
      --default-toolchain "${RUST_TOOLCHAIN}" --component rustfmt --component clippy \
 && rm /tmp/rustup-init.sh \
 && rustc --version && cargo --version && cargo fmt --version && cargo clippy --version \
 && chmod -R a+rX /usr/local/rustup /usr/local/cargo

# Unprivileged build user. Named volumes mounted on /target and the cargo
# registry inherit the ownership of these directories on first use. builder
# also owns CARGO_HOME itself (not its toolchain binaries) so cargo can create
# its package-cache lock and git checkouts there.
RUN useradd --create-home --uid "${BUILDER_UID}" --shell /bin/bash builder \
 && mkdir -p /target /usr/local/cargo/registry /usr/local/cargo/git \
 && chown builder:builder /target /usr/local/cargo /usr/local/cargo/registry /usr/local/cargo/git

USER builder
WORKDIR /repo/clients/linux
