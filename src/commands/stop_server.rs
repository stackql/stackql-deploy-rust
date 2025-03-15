use crate::utils::display::print_unicode_box;
use crate::utils::server::stop_server;
use clap::{Arg, ArgAction, ArgMatches, Command};
use colored::*;
use std::process;

pub fn command() -> Command {
    Command::new("stop-server")
        .about("Stop the stackql server")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .help("Port the server is running on")
                .default_value("5444")
                .action(ArgAction::Set),
        )
}

pub fn execute(matches: &ArgMatches) {
    print_unicode_box("🛑 Stopping stackql server...");

    let port = matches
        .get_one::<String>("port")
        .unwrap_or(&"5444".to_string())
        .parse::<u16>()
        .unwrap_or(5444);

    match stop_server(port) {
        Ok(_) => {
            println!("{}", "Stackql server stopped successfully".green());
        }
        Err(e) => {
            eprintln!("{}", format!("Failed to stop server: {}", e).red());
            process::exit(1);
        }
    }
}
