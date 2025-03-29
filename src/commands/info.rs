use crate::utils::display::print_unicode_box;
use crate::utils::platform::get_platform;
use crate::utils::server::find_all_running_servers;
use crate::utils::stackql::{get_installed_providers, get_stackql_path, get_version};
use clap::Command;
use colored::*;
use std::process;

pub fn command() -> Command {
    Command::new("info").about("Display version information")
}

pub fn execute() {
    print_unicode_box("📋 Getting program information...");

    // Get stackql version
    let version_info = match get_version() {
        Ok(info) => info,
        Err(e) => {
            eprintln!("{}", format!("Error: {}", e).red());
            process::exit(1);
        }
    };

    // Get platform
    let platform = get_platform();

    // Get binary path
    let binary_path = match get_stackql_path() {
        Some(path) => path.to_string_lossy().to_string(),
        _none => "Not found".to_string(),
    };

    // Get all running StackQL servers
    let running_servers = find_all_running_servers();

    // Get installed providers
    let providers = get_installed_providers().unwrap_or_default();

    // Print information
    println!("{}", "stackql-deploy CLI".green().bold());
    println!("  Version: 0.1.0\n");

    println!("{}", "StackQL Library".green().bold());
    println!("  Version: {}", version_info.version);
    println!("  SHA: {}", version_info.sha);
    println!("  Platform: {:?}", platform);
    println!("  Binary Path: {}", binary_path);

    // Display running servers
    println!("\n{}", "Local StackQL Servers".green().bold());
    if running_servers.is_empty() {
        println!("  None");
    } else {
        for server in running_servers {
            println!("  PID: {}, Port: {}", server.pid, server.port);
        }
    }

    // Update the providers display section
    println!("\n{}", "Installed Providers".green().bold());
    if providers.is_empty() {
        println!("  No providers installed");
    } else {
        for provider in providers {
            println!("  {} {}", provider.name.bold(), provider.version);
        }
    }

    // Display contributors
    let raw_contributors = option_env!("CONTRIBUTORS").unwrap_or("");
    let contributors: Vec<&str> = raw_contributors
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .collect();

    if !contributors.is_empty() {
        println!("\n{}", "Special thanks to:".green().bold());

        for chunk in contributors.chunks(5) {
            println!("  {}", chunk.join(", "));
        }
    }
}
