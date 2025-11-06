#!/bin/bash
set -e

echo "Running Nix build in Docker..."
docker run --rm -v "$(pwd)":/app -w /app nixos/nix bash -c "git config --global --add safe.directory /app && nix build --extra-experimental-features 'nix-command flakes'"
echo "Nix build test successful."
