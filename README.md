# Basic build (debug mode)
cargo build

# Release build (optimized with no debug info)
cargo build --release

# Build with verbose output
cargo build -v

# Check if your code compiles without producing an executable
cargo check

# Build and run the application
cargo run

# Build and run with command line arguments
cargo run -- build --env prod --provider aws --region us-east-1


./target/release/stackql-deploy --version

./target/release/stackql-deploy --help

./target/release/stackql-deploy info

./target/release/stackql-deploy init my-stack --provider aws

./target/release/stackql-deploy build my-stack dev

./target/release/stackql-deploy test my-stack dev

./target/release/stackql-deploy teardown my-stack dev

./target/release/stackql-deploy build

./target/release/stackql-deploy unknowncmd

./target/release/stackql-deploy shell

./target/release/stackql-deploy upgrade
