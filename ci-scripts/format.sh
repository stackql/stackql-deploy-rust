#!/bin/bash
set -e

echo "==============================================="
echo "  Running Rust Formatter"
echo "==============================================="

# Run cargo fmt on all crates in workspace
echo "Formatting code with rustfmt..."
cargo fmt --all

echo "✅ Code formatting complete!"