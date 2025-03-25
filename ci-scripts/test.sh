#!/bin/bash
set -e

# Display script banner
echo "==============================================="
echo "  Running Tests for stackql-deploy"
echo "==============================================="

# Run unit tests
echo "Running unit tests..."
cargo test --lib

# Run integration tests if they exist
echo -e "\nRunning integration tests..."
cargo test --test '*'

# Run doc tests
echo -e "\nRunning documentation tests..."
cargo test --doc

echo -e "\n✅ All tests passed successfully!"