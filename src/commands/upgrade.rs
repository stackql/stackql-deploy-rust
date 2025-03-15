use clap::Command;
use std::process::{self, Command as ProcessCommand};
use colored::*;
use crate::utils::display::print_unicode_box;
use crate::utils::download::download_binary;

pub fn command() -> Command {
    Command::new("upgrade").about("Upgrade the CLI and dependencies")
}

pub fn execute() {
    print_unicode_box("📦 Upgrading stackql...");
    
    // Download the latest version of stackql binary
    match download_binary() {
        Ok(path) => {
            println!("Successfully upgraded stackql binary to the latest version at:");
            println!("{}", path.display().to_string().green());
            println!("Upgrade complete!");
        },
        Err(e) => {
            eprintln!("{}", format!("Error upgrading stackql binary: {}", e).red());
            process::exit(1);
        }
    }
}