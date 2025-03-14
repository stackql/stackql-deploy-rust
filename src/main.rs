mod commands;
mod utils;
mod error;

use clap::Command;
use std::process;
use utils::display::{print_error, print_info};

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
        .get_matches();

    // Modify your binary check section
    if !matches.subcommand_name().map_or(false, |name| name == "init") {
        if !utils::binary::binary_exists_in_current_dir() && !utils::binary::binary_exists_in_path() {
            print_info("stackql binary not found in the current directory or in the PATH. Downloading the latest version...");
            
            match utils::download::download_binary() {
                Ok(_) => {
                    print_info("StackQL binary has been successfully downloaded and installed.");
                },
                Err(e) => {
                    print_error(&format!("Failed to download StackQL binary: {}", e));
                    process::exit(1);
                }
            }
        }
    }

    match matches.subcommand() {
        Some(("build", sub_matches)) => commands::build::execute(sub_matches),
        Some(("teardown", sub_matches)) => commands::teardown::execute(sub_matches),
        Some(("test", sub_matches)) => commands::test::execute(sub_matches),
        Some(("info", _)) => commands::info::execute(),
        Some(("shell", _)) => commands::shell::execute(),
        Some(("upgrade", _)) => commands::upgrade::execute(),
        Some(("init", sub_matches)) => commands::init::execute(sub_matches),
        _ => {
            print_error("Unknown command. Use --help for usage.");
            process::exit(1);
        }
    }
}