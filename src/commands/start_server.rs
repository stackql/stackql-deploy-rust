use crate::utils::display::print_unicode_box;
use crate::utils::server::{start_server, ServerOptions};
use clap::{Arg, ArgAction, ArgMatches, Command};
use colored::*;
use std::process;

pub fn command() -> Command {
    Command::new("start-server")
        .about("Start the stackql server")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .help("Port to listen on")
                .default_value("5444")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("registry")
                .short('r')
                .long("registry")
                .help("Custom registry URL")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("arg")
                .short('a')
                .long("arg")
                .help("Additional arguments to pass to stackql")
                .action(ArgAction::Append),
        )
}

pub fn execute(matches: &ArgMatches) {
    print_unicode_box("🚀 Starting stackql server...");

    let port = matches
        .get_one::<String>("port")
        .unwrap_or(&"5444".to_string())
        .parse::<u16>()
        .unwrap_or(5444);

    let registry = matches.get_one::<String>("registry").cloned();

    let additional_args = matches
        .get_many::<String>("arg")
        .map(|vals| vals.cloned().collect())
        .unwrap_or_default();

    let options = ServerOptions {
        port,
        registry,
        additional_args,
    };

    match start_server(&options) {
        Ok(pid) => {
            println!(
                "{}",
                format!("Stackql server started with PID: {}", pid).green()
            );
        }
        Err(e) => {
            eprintln!("{}", format!("Failed to start server: {}", e).red());
            process::exit(1);
        }
    }
}
