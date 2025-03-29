use crate::app::{DEFAULT_SERVER_HOST, DEFAULT_SERVER_PORT};
use once_cell::sync::OnceCell;

// Global static cells that will hold our values
static STACKQL_SERVER_HOST: OnceCell<String> = OnceCell::new();
static STACKQL_SERVER_PORT: OnceCell<u16> = OnceCell::new();
static STACKQL_CONNECTION_STRING: OnceCell<String> = OnceCell::new();

// Initialize the global values
pub fn init_globals(host: String, port: u16) {
    // Only set if not already set (first initialization wins)
    STACKQL_SERVER_HOST.set(host.clone()).ok();
    STACKQL_SERVER_PORT.set(port).ok();

    // Create a connection string and store it globally
    let connection_string = format!(
        "host={} port={} user=stackql dbname=stackql application_name=stackql",
        host, port
    );
    STACKQL_CONNECTION_STRING.set(connection_string).ok();
}

// Getter for the host value
pub fn server_host() -> &'static str {
    STACKQL_SERVER_HOST
        .get()
        .map_or(DEFAULT_SERVER_HOST, |s| s.as_str())
}

// Getter for the port value
pub fn server_port() -> u16 {
    STACKQL_SERVER_PORT
        .get()
        .copied()
        .unwrap_or(DEFAULT_SERVER_PORT)
}

// Getter for the connection string (Returns &str for easier use)
pub fn connection_string() -> &'static str {
    STACKQL_CONNECTION_STRING.get().map_or("", |s| s.as_str())
}
