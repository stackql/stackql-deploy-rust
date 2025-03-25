#!/bin/bash
set -e

# Display script banner
echo "==============================================="
echo "  Building stackql-deploy"
echo "==============================================="

# Build in release mode
echo "Building in release mode..."
cargo build --release

# Check if build was successful
if [ $? -eq 0 ]; then
    echo -e "\n✅ Build completed successfully!"
    echo "Binary location: ./target/release/stackql-deploy"
else
    echo -e "\n❌ Build failed!"
    exit 1
fi

# Create binaries for different platforms if cross-compilation tools are available
if command -v cross &> /dev/null; then
    echo -e "\nCross-compiling for multiple platforms..."
    
    # Build for Windows
    echo "Building for Windows..."
    cross build --release --target x86_64-pc-windows-gnu
    
    # Build for macOS
    echo "Building for macOS..."
    cross build --release --target x86_64-apple-darwin
    
    # Build for Linux
    echo "Building for Linux..."
    cross build --release --target x86_64-unknown-linux-gnu
    
    echo -e "\n✅ Cross-compilation completed!"
    echo "Binaries located in ./target/{target}/release/"
fi