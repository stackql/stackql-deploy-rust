use crate::utils::display::print_unicode_box;
use crate::utils::server::{is_server_running, start_server, ServerOptions};
use crate::utils::stackql::{execute_query_with_pg, QueryResult};
use clap::{Arg, ArgAction, ArgMatches, Command};
use colored::*;
use postgres::Client;
use postgres::NoTls;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::process;

pub fn command() -> Command {
    Command::new("shell")
        .about("Launch the interactive shell")
        .arg(
            Arg::new("port")
                .short('p')
                .long("port")
                .help("Port to connect to")
                .default_value("5444")
                .action(ArgAction::Set),
        )
        .arg(
            Arg::new("host")
                .short('h')
                .long("host")
                .help("Host to connect to")
                .default_value("localhost")
                .action(ArgAction::Set),
        )
}

pub fn execute(matches: &ArgMatches) {
    print_unicode_box("🔗 Launching interactive shell...");

    let port = matches
        .get_one::<String>("port")
        .unwrap_or(&"5444".to_string())
        .parse::<u16>()
        .unwrap_or(5444);

    let localhost = String::from("localhost");
    let host = matches.get_one::<String>("host").unwrap_or(&localhost);

    // Check if server is running, start if not
    if host == "localhost" && !is_server_running(port) {
        println!("{}", "Server not running. Starting server...".yellow());
        let options = ServerOptions {
            port,
            ..Default::default()
        };

        match start_server(&options) {
            Ok(_) => {
                println!("{}", "Server started successfully".green());
            }
            Err(e) => {
                eprintln!("{}", format!("Failed to start server: {}", e).red());
                process::exit(1);
            }
        }
    }

    // Connect to the server
    let connection_string = format!(
        "host={} port={} user=postgres dbname=stackql application_name=stackql",
        host, port
    );
    let _client = match Client::connect(&connection_string, NoTls) {
        Ok(client) => client,
        Err(e) => {
            eprintln!("{}", format!("Failed to connect to server: {}", e).red());
            process::exit(1);
        }
    };

    println!("Connected to stackql server at {}:{}", host, port);
    println!("Type 'exit' to quit the shell");
    println!("---");

    // Set up command history with rustyline
    let mut rl = Editor::<()>::new().unwrap();
    let _ = rl.load_history("stackql_history.txt"); // Silently load history, ignore errors

    // REPL loop
    loop {
        let prompt = format!("stackql ({}:{})=> ", host, port);
        let readline = rl.readline(&prompt);

        match readline {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                // Add to history
                rl.add_history_entry(input);

                if input.eq_ignore_ascii_case("exit") || input.eq_ignore_ascii_case("quit") {
                    println!("Goodbye");
                    break;
                }

                // Execute the query
                match execute_query_with_pg(input, port) {
                    Ok(result) => match result {
                        QueryResult::Data { columns, rows } => {
                            print_table(columns, rows);
                        }
                        QueryResult::Command(cmd) => {
                            println!("{}", format!("Command completed: {}", cmd).green());
                        }
                        QueryResult::Empty => {
                            println!("{}", "Query executed successfully. No results.".green());
                        }
                    },
                    Err(e) => {
                        eprintln!("{}", format!("Error: {}", e).red());
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history("stackql_history.txt"); // Silently save history, ignore errors
}

// Function to print a formatted table
fn print_table(
    columns: Vec<crate::utils::stackql::QueryResultColumn>,
    rows: Vec<crate::utils::stackql::QueryResultRow>,
) {
    // Calculate column widths
    let mut column_widths: Vec<usize> = columns.iter().map(|col| col.name.len()).collect();

    // Update widths based on data
    for row in &rows {
        for (i, value) in row.values.iter().enumerate() {
            if i < column_widths.len() && value.len() > column_widths[i] {
                column_widths[i] = value.len();
            }
        }
    }

    // Print top border
    print!("+");
    for width in &column_widths {
        print!("{}+", "-".repeat(width + 2));
    }
    println!();

    // Print column headers
    print!("|");
    for (i, col) in columns.iter().enumerate() {
        print!(
            " {}{} |",
            col.name,
            " ".repeat(column_widths[i] - col.name.len())
        );
    }
    println!();

    // Print header separator
    print!("+");
    for width in &column_widths {
        print!("{}+", "-".repeat(width + 2));
    }
    println!();

    // Print row data
    let row_count = rows.len();
    for row in rows {
        print!("|");
        for (i, value) in row.values.iter().enumerate() {
            if i < column_widths.len() {
                print!(" {}{} |", value, " ".repeat(column_widths[i] - value.len()));
            }
        }
        println!();
    }

    // Print bottom border
    print!("+");
    for width in &column_widths {
        print!("{}+", "-".repeat(width + 2));
    }
    println!();

    // Print row count
    if row_count > 0 {
        println!("{} rows returned", row_count);
    }
}
