#!/bin/bash
set -e

# Display script banner
echo "==============================================="
echo "  Building stackql-deploy"
echo "==============================================="

# Read contributors into a comma-separated string
CONTRIBS=$(paste -sd, contributors.csv | sed 's/,$//')

# Build in release mode with env var for contributors
echo "Building in release mode with contributors..."
CONTRIBUTORS="$CONTRIBS" cargo rustc --release -- -C link-arg=-s

# Check if build was successful
if [ $? -eq 0 ]; then
    echo -e "\n✅ Build completed successfully!"
    echo "Binary location: ./target/release/stackql-deploy"
else
    echo -e "\n❌ Build failed!"
    exit 1
fi

# Optional: Cross compile
if command -v cross &> /dev/null; then
    echo -e "\nCross-compiling for multiple platforms..."

    echo "Building for Windows..."
    cross build --release --target x86_64-pc-windows-gnu

    echo "Building for macOS..."
    cross build --release --target x86_64-apple-darwin

    echo "Building for Linux..."
    cross build --release --target x86_64-unknown-linux-gnu

    echo -e "\n✅ Cross-compilation completed!"
    echo "Binaries located in ./target/{target}/release/"
fi
