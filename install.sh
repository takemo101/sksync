#!/bin/sh
set -eu

REPO="${SKSYNC_REPO:-takemo101/sksync}"
BIN="sksync"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${VERSION:-latest}"
BASE_URL="https://github.com/$REPO/releases"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

need curl
need tar
need mktemp

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Darwin)
    case "$arch" in
      arm64|aarch64)
        target="aarch64-apple-darwin"
        ;;
      x86_64|amd64)
        target="x86_64-apple-darwin"
        ;;
      *)
        echo "error: unsupported macOS architecture: $arch" >&2
        exit 1
        ;;
    esac
    ;;
  Linux)
    case "$arch" in
      x86_64|amd64)
        target="x86_64-unknown-linux-musl"
        ;;
      arm64|aarch64)
        target="aarch64-unknown-linux-musl"
        ;;
      *)
        echo "error: unsupported Linux architecture: $arch" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "error: unsupported operating system: $os" >&2
    exit 1
    ;;
esac

asset="$BIN-$target.tar.gz"
if [ "$VERSION" = "latest" ]; then
  url="$BASE_URL/latest/download/$asset"
  checksum_url="$BASE_URL/latest/download/checksums.txt"
else
  url="$BASE_URL/download/$VERSION/$asset"
  checksum_url="$BASE_URL/download/$VERSION/checksums.txt"
fi

tmp="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

archive="$tmp/$asset"
echo "Downloading $url"
curl -fsSL "$url" -o "$archive"

checksum() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    return 1
  fi
}

if curl -fsSL "$checksum_url" -o "$tmp/checksums.txt" 2>/dev/null; then
  expected_line="$(grep "  $asset\$" "$tmp/checksums.txt" || true)"
  if [ -n "$expected_line" ]; then
    if actual="$(checksum "$archive")"; then
      expected="$(printf '%s\n' "$expected_line" | awk '{print $1}')"
      if [ "$actual" != "$expected" ]; then
        echo "error: checksum mismatch for $asset" >&2
        echo "expected: $expected" >&2
        echo "actual:   $actual" >&2
        exit 1
      fi
      echo "Checksum verified"
    else
      echo "warning: sha256sum/shasum not found; skipping verification" >&2
    fi
  else
    echo "warning: checksum for $asset not found; skipping verification" >&2
  fi
else
  echo "warning: checksums.txt not found; skipping verification" >&2
fi

tar -xzf "$archive" -C "$tmp"
if [ ! -f "$tmp/$BIN" ]; then
  echo "error: archive did not contain $BIN" >&2
  exit 1
fi

mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp/$BIN" "$INSTALL_DIR/$BIN"

echo "Installed $BIN to $INSTALL_DIR/$BIN"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "warning: $INSTALL_DIR is not on your PATH" >&2 ;;
esac

echo "Run '$BIN --help' to get started."
