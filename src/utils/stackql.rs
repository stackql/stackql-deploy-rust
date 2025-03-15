use crate::utils::binary::get_binary_path;
use crate::utils::server::{is_server_running, start_server, ServerOptions};
use postgres::{Client, NoTls};
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

pub struct VersionInfo {
    pub version: String,
    pub sha: String,
}

pub struct Provider {
    pub name: String,
    pub version: String,
}

pub struct QueryResultColumn {
    pub name: String,
}

pub struct QueryResultRow {
    pub values: Vec<String>,
}

pub enum QueryResult {
    Data {
        columns: Vec<QueryResultColumn>,
        rows: Vec<QueryResultRow>,
    },
    Command(String),
    Empty,
}

/// Get the version information from the stackql binary
pub fn get_version() -> Result<VersionInfo, String> {
    let binary_path = match get_binary_path() {
        Some(path) => path,
        _none => return Err("StackQL binary not found".to_string()),
    };

    let output = match ProcessCommand::new(&binary_path).arg("--version").output() {
        Ok(output) => output,
        Err(e) => return Err(format!("Failed to execute stackql: {}", e)),
    };

    if !output.status.success() {
        return Err("Failed to get version information".to_string());
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let version_line = match output_str.lines().next() {
        Some(line) => line,
        _none => return Err("Empty version output".to_string()),
    };

    let tokens: Vec<&str> = version_line.split_whitespace().collect();
    if tokens.len() < 4 {
        return Err("Unexpected version format".to_string());
    }

    let version = tokens[1].to_string();
    let sha = tokens[3].replace("(", "").replace(")", "");

    Ok(VersionInfo { version, sha })
}

/// Get a list of installed providers
pub fn get_installed_providers() -> Result<Vec<Provider>, String> {
    let binary_path = match get_binary_path() {
        Some(path) => path,
        _none => return Err("StackQL binary not found".to_string()),
    };

    let output = match ProcessCommand::new(&binary_path)
        .arg("exec")
        .arg("SHOW PROVIDERS")
        .output()
    {
        Ok(output) => output,
        Err(e) => return Err(format!("Failed to execute stackql: {}", e)),
    };

    if !output.status.success() {
        return Err("Failed to get providers information".to_string());
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut providers = Vec::new();

    // Parse provider data more carefully
    for line in output_str.lines() {
        if line.contains("name") || line.contains("----") {
            // Skip header and separator lines
            continue;
        }

        let fields: Vec<&str> = line.split('|').collect();
        if fields.len() >= 3 {
            let name = fields[1].trim().to_string();
            let version = fields[2].trim().to_string();
            if !name.is_empty() && name != "name" && !name.contains("----") {
                providers.push(Provider { name, version });
            }
        }
    }

    Ok(providers)
}

/// Execute a SQL query using a Postgres connection to the stackql server
pub fn execute_query_with_pg(query: &str, port: u16) -> Result<QueryResult, String> {
    // Check if server is running, start if not
    if !is_server_running(port) {
        let options = ServerOptions {
            port,
            ..Default::default()
        };
        start_server(&options).map_err(|e| format!("Failed to start server: {}", e))?;
    }

    // Connect to the server
    let connection_string = format!(
        "host=localhost port={} user=postgres dbname=stackql application_name=stackql",
        port
    );
    let mut client = Client::connect(&connection_string, NoTls)
        .map_err(|e| format!("Failed to connect to server: {}", e))?;

    // Try to execute the query directly
    match client.simple_query(query) {
        Ok(results) => {
            let mut columns = Vec::new();
            let mut rows = Vec::new();

            // Process results
            for result in &results {
                if let postgres::SimpleQueryMessage::Row(row) = result {
                    // First row? Extract column information
                    if columns.is_empty() {
                        for i in 0..row.len() {
                            columns.push(QueryResultColumn {
                                name: row.columns()[i].name().to_string(),
                            });
                        }
                    }

                    // Process row data
                    let mut row_values = Vec::new();
                    for i in 0..row.len() {
                        row_values.push(row.get(i).unwrap_or("NULL").to_string());
                    }

                    rows.push(QueryResultRow { values: row_values });
                }
            }

            if !columns.is_empty() {
                Ok(QueryResult::Data { columns, rows })
            } else {
                // Check for command completion
                for result in results {
                    if let postgres::SimpleQueryMessage::CommandComplete(cmd) = result {
                        return Ok(QueryResult::Command(cmd.to_string()));
                    }
                }

                Ok(QueryResult::Empty)
            }
        }
        Err(e) => Err(format!("Query execution failed: {}", e)),
    }
}

/// Execute a SQL query with stackql and return the output as a structured result
#[allow(dead_code)]
pub fn execute_query(query: &str) -> Result<QueryResult, String> {
    execute_query_with_pg(query, 5444)
}

/// Get the binary path
pub fn get_stackql_path() -> Option<PathBuf> {
    get_binary_path()
}
