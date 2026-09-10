#!/bin/bash
set -e

# Display script banner
echo "==============================================="
echo "  Running Tests for stackql-deploy"
echo "==============================================="

# Run unit tests (in-module #[cfg(test)] tests in src/)
echo "Running unit tests..."
cargo test --lib

# Run integration tests (tests/*.rs). These drive the build and teardown
# flows against an in-process mock stackql server - no stackql binary,
# provider registry, network access, or cloud credentials required.
echo -e "\nRunning integration tests..."
cargo test --test '*'

echo -e "\n✅ All tests passed successfully!"
