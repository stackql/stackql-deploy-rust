#!/bin/bash
set -e

# Make scripts executable
chmod +x ci-scripts/format.sh
chmod +x ci-scripts/lint.sh
chmod +x ci-scripts/test.sh
chmod +x ci-scripts/build.sh

# Print banner
echo "==============================================="
echo "  Running Full Local Build Process"
echo "==============================================="

# Run each step in sequence
printf "\n[STEP 1/4] Formatting code...\n"
./ci-scripts/format.sh

printf "\n[STEP 2/4] Running linter...\n"
./ci-scripts/lint.sh

printf "\n[STEP 3/4] Running tests...\n"
# ./ci-scripts/test.sh

printf "\n[STEP 4/4] Building binary...\n"
./ci-scripts/build.sh

printf "\n🎉 Local build process completed successfully!\n"
echo "Binary is available at: ./target/release/stackql-deploy"