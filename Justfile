# Justfile for UnifiedLab

set shell := ["bash", "-c"]

default:
    @just --list

# --- Setup ---

# Install dependencies for Rust and Python
setup:
    @echo "Setting up Rust..."
    cargo check
    @echo "Setting up Python..."
    uv sync

# --- Development ---

# Check code for errors
check:
    cargo check
    uv run ruff check . || true

# Build the project
build:
    cargo build --release

# Run unit tests
test:
    cargo test

# Format code
fmt:
    cargo fmt
    uv run ruff format . || true

# --- Execution ---

# Start the node service (Guardian + Coordinator)
start:
    cargo run -- start --force-local

# Deploy a blueprint (uses mock generator by default for now)
deploy file="experiment.drawio":
    cargo run -- deploy --file {{file}}

# Launch the TUI dashboard
tui:
    cargo run -- tui

# Clean build artifacts
clean:
    cargo clean
    rm -rf .venv
