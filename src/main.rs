mod commands;
mod error;
mod utils;

use crate::utils::display::{print_error, print_info};
use crate::utils::server::stop_server;
use clap::Command;
use error::{get_binary_path_with_error, AppError};
use std::process;

fn main() {
    let matches = Command::new("stackql-deploy")
        .version("0.1.0")
        .author("Jeffrey Aven <javen@stackql.io>")
        .about("Model driven IaC using stackql")
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

    // Check for binary existence except for init and server management commands
    let exempt_commands = ["init"];
    if !exempt_commands.contains(&matches.subcommand_name().unwrap_or("")) {
        if let Err(AppError::BinaryNotFound) = get_binary_path_with_error() {
            print_info("stackql binary not found in the current directory or in the PATH. Downloading the latest version...");
            // Call your download code here
            process::exit(1);
        }
        // if let None = get_binary_path() {
        //     print_info("stackql binary not found in the current directory or in the PATH. Downloading the latest version...");
        //     // Call your download code here
        //     process::exit(1);
        // }
    }

    // Define which commands need server management
    let server_commands = ["build", "test", "plan", "teardown", "shell"];
    let needs_server = server_commands.contains(&matches.subcommand_name().unwrap_or(""));
    let default_port = 5444;

    // Handle command execution
    match matches.subcommand() {
        Some(("build", sub_matches)) => {
            commands::build::execute(sub_matches);
            if needs_server {
                stop_server(default_port).ok();
            }
        }
        Some(("teardown", sub_matches)) => {
            commands::teardown::execute(sub_matches);
            if needs_server {
                stop_server(default_port).ok();
            }
        }
        Some(("test", sub_matches)) => {
            commands::test::execute(sub_matches);
            if needs_server {
                stop_server(default_port).ok();
            }
        }
        Some(("info", _)) => commands::info::execute(),
        Some(("shell", sub_matches)) => commands::shell::execute(sub_matches),
        Some(("upgrade", _)) => commands::upgrade::execute(),
        Some(("init", sub_matches)) => commands::init::execute(sub_matches),
        Some(("start-server", sub_matches)) => commands::start_server::execute(sub_matches),
        Some(("stop-server", sub_matches)) => commands::stop_server::execute(sub_matches),
        Some(("plan", _)) => {
            commands::plan::execute();
            if needs_server {
                stop_server(default_port).ok();
            }
        }
        _ => {
            print_error("Unknown command. Use --help for usage.");
            process::exit(1);
        }
    }
}
