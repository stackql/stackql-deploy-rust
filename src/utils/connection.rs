use crate::globals::connection_string;
use colored::*;
use postgres::{Client, NoTls};
use std::process;

/// Create a new Client connection
pub fn create_client() -> Client {
    let conn_str = connection_string(); // Uses your global connection string
    Client::connect(conn_str, NoTls).unwrap_or_else(|e| {
        eprintln!("{}", format!("Failed to connect to server: {}", e).red());
        process::exit(1); // Exit the program if connection fails, so there's no returning a Result.
    })
}
