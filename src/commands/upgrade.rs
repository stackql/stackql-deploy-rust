use clap::Command;
use std::process;
use colored::*;
use crate::utils::display::print_unicode_box;
use crate::utils::download::download_binary;
use crate::utils::stackql::get_version;

pub fn command() -> Command {
    Command::new("upgrade").about("Upgrade stackql to the latest version")
}

pub fn execute() {
    print_unicode_box("📦 Upgrading stackql...");
    
    // Download the latest version of stackql binary
    match download_binary() {
        Ok(path) => {
            // Get the version of the newly installed binary
            match get_version() {
                Ok(version_info) => {
                    println!("Successfully upgraded stackql binary to the latest version ({}) at:", version_info.version);
                },
                Err(_) => {
                    println!("Successfully upgraded stackql binary to the latest version at:");
                }
            }
            println!("{}", path.display().to_string().green());
            println!("Upgrade complete!");
        },
        Err(e) => {
            eprintln!("{}", format!("Error upgrading stackql binary: {}", e).red());
            process::exit(1);
        }
    }
}