use crate::app::LOCAL_SERVER_ADDRESSES;
use crate::globals::{server_host, server_port};
use crate::utils::display::print_unicode_box;
use crate::utils::server::{is_server_running, start_server, StartServerOptions};
use clap::{Arg, ArgAction, ArgMatches, Command};
use colored::*;
use std::process;

pub fn command() -> Command {
    Command::new("start-server")
        .about("Start the stackql server")
        .arg(
            Arg::new("registry")
                .short('r')
                .long("registry")
                .help("[OPTIONAL] Custom registry URL")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("mtls_config")
                .short('m')
                .long("mtls-config")
                .help("[OPTIONAL] mTLS configuration for the server (JSON object)")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("custom_auth_config")
                .short('a')
                .long("custom-auth-config")
                .help("[OPTIONAL] Custom provider authentication configuration for the server (JSON object)")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("log_level")
                .short('l')
                .long("log-level")
                .help("[OPTIONAL] Server log level (default: WARN)")
                .value_parser(["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"])
                .action(ArgAction::Set),
        )
}

pub fn execute(matches: &ArgMatches) {
    print_unicode_box("🚀 Starting stackql server...");

    // Parse port from args or use default
    let port = matches
        .get_one::<String>("port")
        .unwrap_or(&server_port().to_string())
        .parse::<u16>()
        .unwrap_or_else(|_| {
            eprintln!("{}", "Invalid port number. Using default.".yellow());
            server_port()
        });

    // Parse host from args or use default
    let host = matches
        .get_one::<String>("host")
        .unwrap_or(&server_host().to_string())
        .to_string();

    // Validate host - must be localhost or 0.0.0.0
    if !LOCAL_SERVER_ADDRESSES.contains(&host.as_str()) {
        eprintln!(
            "{}",
            "Error: Host must be 'localhost' or '0.0.0.0' for local server setup.".red()
        );
        eprintln!("The start-server command is only for starting a local server instance.");
        process::exit(1);
    }

    // Check if server is already running
    if is_server_running(port) {
        println!(
            "{}",
            format!(
                "Server is already running on port {}. No action needed.",
                port
            )
            .yellow()
        );
        process::exit(0);
    }

    // Get optional settings
    let registry = matches.get_one::<String>("registry").cloned();
    let mtls_config = matches.get_one::<String>("mtls_config").cloned();
    let custom_auth_config = matches.get_one::<String>("custom_auth_config").cloned();
    let log_level = matches.get_one::<String>("log_level").cloned();

    // Create server options
    let options = StartServerOptions {
        host,
        port,
        registry,
        mtls_config,
        custom_auth_config,
        log_level,
    };

    // Start the server
    match start_server(&options) {
        Ok(pid) => {
            println!(
                "{}",
                format!("Stackql server started with PID: {}", pid).green()
            );
            println!(
                "{}",
                format!("Server is listening on {}:{}", options.host, options.port).green()
            );
        }
        Err(e) => {
            eprintln!("{}", format!("Failed to start server: {}", e).red());
            process::exit(1);
        }
    }
}
