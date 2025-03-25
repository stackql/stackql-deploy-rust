#!/bin/bash
set -e

# Display script banner
echo "==============================================="
echo "  Running Rust Linter (clippy)"
echo "==============================================="

# Run clippy with warning-level lints
cargo clippy -- -D warnings

# Run rustfmt to check formatting
echo -e "\nChecking code formatting with rustfmt..."
cargo fmt -- --check

echo -e "\n✅ Linting passed successfully!"