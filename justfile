# Scientific Calculator MCP App — task runner.
#
# Usage: `just <task>`. Run `just` with no args for the list.

# Default: list available tasks.
default:
    @just --list

# Install widget npm dependencies (one-time, before first build).
install-widgets:
    npm --prefix widgets install

# Build the widget HTML bundles (keypad + step) into widgets/dist/.
# Run this whenever widgets/keypad.html or widgets/step.html changes.
build-widgets:
    npm --prefix widgets run build

# Build the Rust crate. Depends on the widget bundles since src/lib.rs
# include_str!s widgets/dist/*.html at compile time.
build: build-widgets
    cargo build --release

# Run the local HTTP MCP server on port 3000 (override with PORT env).
run: build-widgets
    cargo run --release

# Run the full Rust test suite.
test:
    cargo test

# Build the AWS Lambda bootstrap binary (ARM64). Requires `cargo lambda`.
build-lambda: build-widgets
    cargo lambda build --release --arm64 --manifest-path scientific-calculator-mcp-app-lambda/Cargo.toml

# Quick rebuild after editing only the widgets (skips cargo).
alias widgets := build-widgets

# Clean everything (Rust target + widget dist + node_modules).
clean:
    cargo clean
    rm -rf widgets/dist widgets/node_modules
