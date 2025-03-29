mod app;
mod commands;
mod error;
mod globals;
mod utils;

use crate::app::{
    APP_AUTHOR, APP_DESCRIPTION, APP_NAME, APP_VERSION, DEFAULT_SERVER_HOST, DEFAULT_SERVER_PORT,
    EXEMPT_COMMANDS,
};
use crate::utils::display::{print_error, print_info};
use clap::{Arg, ArgAction, Command};
use error::{get_binary_path_with_error, AppError};
use std::process;

fn main() {
    let matches = Command::new(APP_NAME)
        .version(APP_VERSION)
        .author(APP_AUTHOR)
        .about(APP_DESCRIPTION)
        // global flags
        .arg(
            Arg::new("server")
                .long("server")
                .alias("host") // Add --host as another option
                .short('h') // Add -h shorthand
                .help("Server host to connect to")
                .global(true) // Make it available to all subcommands
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .help("Server port to connect to (1024-65535)")
                .value_parser(clap::value_parser!(u16).range(1024..=65535))
                .global(true) // Make it available to all subcommands
                .action(ArgAction::Set),
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
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

    // Get the server and port values from command-line arguments
    let server_host = matches
        .get_one::<String>("server")
        .unwrap_or(&DEFAULT_SERVER_HOST.to_string())
        .clone();

    let server_port = *matches
        .get_one::<u16>("port")
        .unwrap_or(&DEFAULT_SERVER_PORT);

    // Initialize the global values
    globals::init_globals(server_host, server_port);

    // Check for binary existence except for exempt commands
    if !EXEMPT_COMMANDS.contains(&matches.subcommand_name().unwrap_or("")) {
        if let Err(AppError::BinaryNotFound) = get_binary_path_with_error() {
            print_info("stackql binary not found in the current directory or in the PATH. Downloading the latest version...");
            // Call your download code here
            process::exit(1);
        }
    }

    // Handle command execution
    match matches.subcommand() {
        Some(("build", sub_matches)) => commands::build::execute(sub_matches),
        Some(("teardown", sub_matches)) => commands::teardown::execute(sub_matches),
        Some(("test", sub_matches)) => commands::test::execute(sub_matches),
        Some(("plan", _)) => commands::plan::execute(),
        Some(("info", _)) => commands::info::execute(),
        Some(("shell", sub_matches)) => commands::shell::execute(sub_matches),
        Some(("upgrade", _)) => commands::upgrade::execute(),
        Some(("init", sub_matches)) => commands::init::execute(sub_matches),
        Some(("start-server", sub_matches)) => commands::start_server::execute(sub_matches),
        Some(("stop-server", sub_matches)) => commands::stop_server::execute(sub_matches),
        _ => {
            print_error("Unknown command. Use --help for usage.");
            process::exit(1);
        }
    }
}
