use std::process::{Command as ProcessCommand};
use std::path::PathBuf;
use crate::utils::binary::get_binary_path;

pub struct VersionInfo {
    pub version: String,
    pub sha: String,
}

pub struct Provider {
    pub name: String,
    pub version: String,
}

/// Get the version information from the stackql binary
pub fn get_version() -> Result<VersionInfo, String> {
    let binary_path = match get_binary_path() {
        Some(path) => path,
        None => return Err("StackQL binary not found".to_string()),
    };
    
    let output = match ProcessCommand::new(&binary_path)
        .arg("--version")
        .output() {
            Ok(output) => output,
            Err(e) => return Err(format!("Failed to execute stackql: {}", e)),
        };
    
    if !output.status.success() {
        return Err("Failed to get version information".to_string());
    }
    
    let output_str = String::from_utf8_lossy(&output.stdout);
    let version_line = match output_str.lines().next() {
        Some(line) => line,
        None => return Err("Empty version output".to_string()),
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
        None => return Err("StackQL binary not found".to_string()),
    };
    
    let output = match ProcessCommand::new(&binary_path)
        .arg("exec")
        .arg("SHOW PROVIDERS")
        .output() {
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

/// Execute a SQL query with stackql and return the output as a string
pub fn execute_query(query: &str) -> Result<String, String> {
    let binary_path = match get_binary_path() {
        Some(path) => path,
        None => return Err("StackQL binary not found".to_string()),
    };
    
    let output = match ProcessCommand::new(&binary_path)
        .arg("exec")
        .arg(query)
        .output() {
            Ok(output) => output,
            Err(e) => return Err(format!("Failed to execute stackql: {}", e)),
        };
    
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Query execution failed: {}", error));
    }
    
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Get the binary path
pub fn get_stackql_path() -> Option<PathBuf> {
    get_binary_path()
}