#!/bin/sh
# Build step for `herdr plugin install chrisg32/tsk`.
#
# Leaves a `tsk` binary at bin/tsk. Prefers the prebuilt release matching this
# checkout's version and platform so users don't need a Rust toolchain; falls
# back to `cargo build --release` when no asset exists (or when
# TSK_BUILD_FROM_SOURCE=1 is set).
set -eu
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case "$(uname -s)" in
  Darwin) target_os=apple-darwin ;;
  Linux) target_os=unknown-linux-gnu ;;
  *) target_os= ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) target_arch=aarch64 ;;
  x86_64 | amd64) target_arch=x86_64 ;;
  *) target_arch= ;;
esac

mkdir -p bin

if [ -n "$target_os" ] && [ -n "$target_arch" ] && [ "${TSK_BUILD_FROM_SOURCE:-}" != "1" ]; then
  asset="tsk-v${version}-${target_arch}-${target_os}.tar.gz"
  url="https://github.com/chrisg32/tsk/releases/download/v${version}/${asset}"
  tmp=$(mktemp -d)
  if curl -fsSL "$url" -o "$tmp/$asset" 2>/dev/null \
    && tar -xzf "$tmp/$asset" -C "$tmp" \
    && [ -x "$tmp/tsk" ]; then
    mv "$tmp/tsk" bin/tsk
    rm -rf "$tmp"
    echo "tsk: installed prebuilt $asset"
    bin/tsk --version
    exit 0
  fi
  rm -rf "$tmp"
  echo "tsk: no prebuilt binary for v${version} ${target_arch}-${target_os}; building from source"
fi

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
  PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "tsk: cargo not found. Install Rust from https://rustup.rs and re-run the install." >&2
  exit 1
fi

cargo build --release --locked
cp target/release/tsk bin/tsk
echo "tsk: built from source"
bin/tsk --version
