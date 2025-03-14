use clap::Command;
use std::process::{self, Command as ProcessCommand};
use colored::*;
use crate::utils::display::print_unicode_box;
use crate::utils::binary::get_binary_path;

pub fn command() -> Command {
    Command::new("shell").about("Launch the interactive shell")
}

pub fn execute() {
    print_unicode_box("🔗 Launching interactive shell...");
    
    // Find the stackql binary path
    let binary_path = match get_binary_path() {
        Some(path) => path,
        None => {
            eprintln!("{}", "Error: StackQL binary not found in the current directory or PATH.".red());
            process::exit(1);
        }
    };
    
    println!("Launching stackql shell from: {}", binary_path.display().to_string().blue());
    
    // Launch the stackql shell as a subprocess
    let result = ProcessCommand::new(&binary_path)
        .arg("shell")
        .arg("--colorscheme")
        .arg("null")
        .status();
    
    match result {
        Ok(status) if !status.success() => {
            eprintln!("{}", format!("Error launching stackql shell. Exit code: {}", status).red());
            process::exit(status.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("{}", format!("Error launching stackql shell: {}", e).red());
            process::exit(1);
        }
        _ => {
            // Shell exited successfully
        }
    }
}