// main.rs

//! # StackQL Deploy - Main Entry Point
//!
//! This is the main entry point for the StackQL Deploy application.
//! It initializes the CLI, configures global settings, and handles user commands (e.g., `build`, `teardown`, `test`, `info`, `shell`, etc.).
//!
//! ## Global Arguments
//!
//! These arguments can be specified for **any command**.
//!
//! - `--server`, `-h` - The server host to connect to (default: `localhost`).
//! - `--port`, `-p` - The server port to connect to (default: `5444`).
//! - `--log-level` - The logging level (default: `info`). Possible values: `error`, `warn`, `info`, `debug`, `trace`.
//!
//! ## Example Usage
//! ```bash
//! ./stackql-deploy --server myserver.com --port 1234 build
//! ./stackql-deploy shell -h localhost -p 5444
//! ./stackql-deploy info
//! ```
//!
//! For detailed help, use `--help` or `-h` flags.

use std::process;

use clap::{Arg, ArgAction, Command};

use log::{debug, error, info};

use stackql_deploy::app::{
    APP_AUTHOR, APP_DESCRIPTION, APP_NAME, APP_VERSION, DEFAULT_LOG_LEVEL, DEFAULT_SERVER_HOST,
    DEFAULT_SERVER_PORT, DEFAULT_SERVER_PORT_STR, EXEMPT_COMMANDS, LOG_LEVELS,
};
use stackql_deploy::error::{get_binary_path_with_error, AppError};
use stackql_deploy::utils::logging::initialize_logger;
use stackql_deploy::{commands, globals, print_error};

/// Main function that initializes the CLI and handles command execution.
fn main() {
    let matches = Command::new(APP_NAME)
        .version(APP_VERSION)
        .author(APP_AUTHOR)
        .about(APP_DESCRIPTION)
        // ====================
        // Global Flags
        // ====================
        .arg(
            Arg::new("server")
                .long("server")
                .alias("host")
                .short('H')
                .help("StackQL server host to connect to")
                .global(true)
                .default_value(DEFAULT_SERVER_HOST)
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .help("StackQL server port to connect to")
                .value_parser(clap::value_parser!(u16).range(1024..=65535))
                .global(true)
                .default_value(DEFAULT_SERVER_PORT_STR)
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("log-level")
                .long("log-level")
                .help("Set the logging level")
                .global(true)
                .value_parser(clap::builder::PossibleValuesParser::new(LOG_LEVELS))
                .ignore_case(true)
                .default_value(DEFAULT_LOG_LEVEL)
                .action(ArgAction::Set),
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        // ====================
        // Subcommand Definitions
        // ====================
        .subcommand(commands::build::command())
        .subcommand(commands::teardown::command())
        .subcommand(commands::test::command())
        .subcommand(commands::info::command())
        .subcommand(commands::shell::command())
        .subcommand(commands::upgrade::command())
        .subcommand(commands::init::command())
        .subcommand(commands::start_server::command())
        .subcommand(commands::stop_server::command())
        .subcommand(commands::plan::command())
        .get_matches();

    // ====================
    // Initialize Logger
    // ====================
    let log_level = matches.get_one::<String>("log-level").unwrap();
    initialize_logger(log_level);

    debug!("Logger initialized with level: {}", log_level);

    // Get the server and port values from command-line arguments
    let server_host = matches
        .get_one::<String>("server")
        .unwrap_or(&DEFAULT_SERVER_HOST.to_string())
        .clone();

    let server_port = *matches
        .get_one::<u16>("port")
        .unwrap_or(&DEFAULT_SERVER_PORT);

    debug!("Server Host: {}", server_host);
    debug!("Server Port: {}", server_port);

    // Initialize the global values
    globals::init_globals(server_host, server_port);

    // Check for binary existence except for exempt commands
    if !EXEMPT_COMMANDS.contains(&matches.subcommand_name().unwrap_or("")) {
        match get_binary_path_with_error() {
            Ok(path) => debug!("StackQL binary found at: {:?}", path),
            Err(_e) => {
                info!("StackQL binary not found. Downloading the latest version...");
                commands::upgrade::execute();

                // Re-check for binary existence after upgrade attempt
                if let Err(AppError::BinaryNotFound) = get_binary_path_with_error() {
                    error!("Failed to download StackQL binary. Please try again or check your network connection.");
                    process::exit(1);
                }
            }
        }
    }

    // ====================
    // Command Execution
    // ====================
    match matches.subcommand() {
        Some(("build", sub_matches)) => commands::build::execute(sub_matches),
        Some(("test", sub_matches)) => commands::test::execute(sub_matches),
        Some(("plan", sub_matches)) => commands::plan::execute(sub_matches),
        Some(("teardown", sub_matches)) => commands::teardown::execute(sub_matches),
        Some(("info", _)) => commands::info::execute(),
        Some(("shell", sub_matches)) => commands::shell::execute(sub_matches),
        Some(("upgrade", _)) => commands::upgrade::execute(),
        Some(("init", sub_matches)) => commands::init::execute(sub_matches),
        Some(("start-server", sub_matches)) => commands::start_server::execute(sub_matches),
        Some(("stop-server", sub_matches)) => commands::stop_server::execute(sub_matches),
        _ => {
            print_error!("Unknown command. Use --help for usage.");
            process::exit(1);
        }
    }
}
