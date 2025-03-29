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
    let port = server_port();

    print_unicode_box("🛑 Stopping stackql server...");

    println!("{}", format!("Stopping server on port {}", port).yellow());

    match stop_server(port) {
        Ok(_) => {
            println!("{}", "StackQL server stopped successfully".green());
        }
        Err(e) => {
            eprintln!("{}", format!("Failed to stop server: {}", e).red());
            process::exit(1);
        }
    }
}
