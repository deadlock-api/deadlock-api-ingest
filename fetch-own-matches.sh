#!/bin/sh
# Recover salts for your own Deadlock matches via the Steam Game Coordinator.
# Usage: curl -fsSL https://raw.githubusercontent.com/deadlock-api/deadlock-api-ingest/master/fetch-own-matches.sh | sh
set -eu

case "$(uname -s)" in
    Linux) asset=deadlock-api-ingest-ubuntu-latest ;;
    Darwin) asset=deadlock-api-ingest-macos ;;
    *) echo "Only Linux and macOS are supported; on Windows use fetch-own-matches.ps1" >&2; exit 1 ;;
esac

bin=$(mktemp)
trap 'rm -f "$bin"' EXIT
curl -fsSL -o "$bin" "https://github.com/deadlock-api/deadlock-api-ingest/releases/latest/download/$asset"
chmod +x "$bin"
RUST_LOG=info "$bin" --own-matches
