# Common tasks. Install `just` (https://just.systems), then run `just` to list them.
# Each recipe is a plain command, so you can also copy it into a terminal.

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

# List the recipes
default:
    @just --list

# Download the face models and ONNX Runtime (checked against their SHA-256)
[unix]
fetch:
    scripts/fetch-models.sh
    scripts/fetch-onnxruntime.sh

# Download the face models and ONNX Runtime (checked against their SHA-256)
[windows]
fetch:
    powershell -ExecutionPolicy Bypass -File scripts\fetch-models.ps1
    powershell -ExecutionPolicy Bypass -File scripts\fetch-onnxruntime.ps1

# Run the gallery on the sample photo, with a temporary index (or pass folders)
[unix]
dev *folders="fixtures/photos":
    #!/usr/bin/env bash
    set -euo pipefail
    data=$(mktemp -d)
    trap 'rm -rf "$data"' EXIT
    cargo run --release -- --data "$data" {{folders}}

# Formatting and lints, as CI checks them
[unix]
lint:
    cargo fmt --all --check
    cargo clippy --locked --all-targets -- -D warnings
    cd web/tests && npm ci --silent && npm run check-scripts

# Format the code
fmt:
    cargo fmt --all

# The Rust tests
test:
    cargo test --locked

# The browser tests (Node 24 or newer; installs Chromium the first time)
ui-test:
    cargo build --locked --release
    cd web/tests && npm ci && npx playwright install chromium && npx playwright test

# Dependency licenses and advisories (needs cargo-deny)
deny:
    cargo deny check

# Everything CI runs, except the other systems and the minimum Rust version
[unix]
check: lint test deny ui-test

# Build the desktop app (Linux: AppImage; Windows: exe), after `just fetch`
[linux]
desktop:
    cd desktop && cargo tauri build --bundles appimage

# Build the desktop app (Linux: AppImage; Windows: exe), after `just fetch`
[windows]
desktop:
    cargo build --release -p imadive-desktop --features custom-protocol

# Prepare a release: bump the version, commit and tag (see docs/releasing.md)
[unix]
release version:
    scripts/release.sh {{version}}
