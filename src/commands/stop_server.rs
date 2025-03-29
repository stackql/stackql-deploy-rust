use crate::globals::server_port;
use crate::utils::display::print_unicode_box;
use crate::utils::server::stop_server;
use clap::{ArgMatches, Command};
use colored::*;
use std::process;

pub fn command() -> Command {
    Command::new("stop-server").about("Stop the stackql server")
}

pub fn execute(_matches: &ArgMatches) {
    print_unicode_box("🛑 Stopping stackql server...");

    match stop_server(server_port()) {
        Ok(_) => {
            println!("{}", "Stackql server stopped successfully".green());
        }
        Err(e) => {
            eprintln!("{}", format!("Failed to stop server: {}", e).red());
            process::exit(1);
        }
    }
}
