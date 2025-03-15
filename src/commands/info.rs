use clap::Command;
use std::process;
use colored::*;
use crate::utils::display::print_unicode_box;
use crate::utils::platform::get_platform;
use crate::utils::stackql::{get_version, get_installed_providers, get_stackql_path};

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
        None => "Not found".to_string(),
    };
    
    // Get installed providers
    let providers = match get_installed_providers() {
        Ok(provs) => provs,
        Err(_) => Vec::new(),
    };
    
    // Print information
    println!("{}", "stackql-deploy CLI".green().bold());
    println!("  Version: 0.1.0\n");
    
    println!("{}", "StackQL Library".green().bold());
    println!("  Version: {}", version_info.version);
    println!("  Platform: {:?}", platform);
    println!("  Binary Path: {}\n", binary_path);
    
    // Update the providers display section
    println!("{}", "Installed Providers".green().bold());
    if providers.is_empty() {
        println!("  No providers installed");
    } else {
        for provider in providers {
            println!("  {} {}", provider.name.bold(), provider.version);
        }
    }
}