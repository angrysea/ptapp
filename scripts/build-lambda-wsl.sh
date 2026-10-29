#!/usr/bin/env bash
# Builds the Lambda binaries (api and the hourly sweep) for arm64 inside WSL
# Ubuntu and copies them to target/lambda/{api,sweep}/bootstrap, where
# template.yaml expects them. Use it on Windows when Smart App Control blocks
# Rust's build scripts (os error 4551): Linux programs aren't checked by it.
# scripts/deploy.sh runs it when LAMBDA_BUILD=wsl (see docs/DEPLOY.md).
#
# One-time setup inside Ubuntu (Rust from rustup already installed):
#   rustup target add aarch64-unknown-linux-gnu
#   python3 -m venv ~/.local/share/cargo-lambda
#   ~/.local/share/cargo-lambda/bin/pip install cargo-lambda   # brings zig
set -euo pipefail
source "$HOME/.cargo/env"
export PATH="$HOME/.local/share/cargo-lambda/bin:$PATH"
cd "$(dirname "$0")/.."
# The build cache lives on the Linux disk: much faster than /mnt, and kept
# apart from the Windows build in ./target.
export CARGO_TARGET_DIR="$HOME/.cache/ptapp-target"
for bin in api sweep; do
  cargo lambda build --release --arm64 --bin "$bin"
  mkdir -p "target/lambda/$bin"
  cp "$CARGO_TARGET_DIR/lambda/$bin/bootstrap" "target/lambda/$bin/bootstrap"
done
ls -l target/lambda/*/bootstrap
