use crate::app::DEFAULT_LOG_FILE;
use crate::utils::binary::get_binary_path;
use colored::*;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::Duration;

pub struct StartServerOptions {
    pub host: String,
    pub port: u16,
    pub registry: Option<String>,
    pub mtls_config: Option<String>,
    pub custom_auth_config: Option<String>,
    pub log_level: Option<String>,
}

impl Default for StartServerOptions {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: crate::app::DEFAULT_SERVER_PORT,
            registry: None,
            mtls_config: None,
            custom_auth_config: None,
            log_level: None,
        }
    }
}

/// Check if the stackql server is running
pub fn is_server_running(port: u16) -> bool {
    // Check using process name and port
    if cfg!(target_os = "windows") {
        let output = ProcessCommand::new("tasklist")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute tasklist"));

        let output_str = String::from_utf8_lossy(&output.stdout);
        output_str.contains("stackql") && output_str.contains(&port.to_string())
    } else {
        // Try multiple pattern variations to be more robust
        let patterns = [
            format!("stackql.*--pgsrv.port {}", port),
            format!("stackql.*--pgsrv.port={}", port),
            format!("stackql.*pgsrv.port {}", port),
            format!("stackql.*pgsrv.port={}", port),
        ];

        for pattern in patterns {
            let output = ProcessCommand::new("pgrep")
                .arg("-f")
                .arg(&pattern)
                .output();

            if let Ok(output) = output {
                if !output.stdout.is_empty() {
                    return true;
                }
            }
        }

        // Fallback: Just check for any stackql process
        let output = ProcessCommand::new("pgrep")
            .arg("-f")
            .arg("stackql")
            .output();

        if let Ok(output) = output {
            if !output.stdout.is_empty() {
                // Further check if this is likely our server by examining the process details
                let stdout_content = String::from_utf8_lossy(&output.stdout);
                let pid = stdout_content.trim();

                let ps_output = ProcessCommand::new("ps")
                    .arg("-p")
                    .arg(pid)
                    .arg("-o")
                    .arg("args")
                    .output();

                if let Ok(ps_output) = ps_output {
                    let ps_str = String::from_utf8_lossy(&ps_output.stdout);
                    return ps_str.contains(&port.to_string()) && ps_str.contains("srv");
                }
            }
        }

        false
    }
}

/// Get the PID of the running stackql server
pub fn get_server_pid(port: u16) -> Option<u32> {
    if cfg!(target_os = "windows") {
        let output = ProcessCommand::new("wmic")
            .arg("process")
            .arg("where")
            .arg(format!(
                "CommandLine like '%stackql%--pgsrv.port={}%'",
                port
            ))
            .arg("get")
            .arg("ProcessId")
            .output()
            .ok()?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();
        if lines.len() >= 2 {
            lines[1].trim().parse::<u32>().ok()
        } else {
            None
        }
    } else {
        // For Linux/macOS, let's try multiple pattern variations
        let patterns = [
            format!("stackql.*--pgsrv.port {}", port),
            format!("stackql.*--pgsrv.port={}", port),
            format!("stackql.*pgsrv.port {}", port),
            format!("stackql.*pgsrv.port={}", port),
        ];

        for pattern in patterns {
            let output = ProcessCommand::new("pgrep")
                .arg("-f")
                .arg(&pattern)
                .output()
                .ok()?;

            if !output.stdout.is_empty() {
                let stdout_content = String::from_utf8_lossy(&output.stdout);
                let pid_str = stdout_content.trim();
                if let Ok(pid) = pid_str.parse::<u32>() {
                    return Some(pid);
                }
            }
        }

        // Try a more general approach to find the stackql server
        let output = ProcessCommand::new("pgrep")
            .arg("-f")
            .arg("stackql.*srv")
            .output()
            .ok()?;

        if !output.stdout.is_empty() {
            let stdout_content = String::from_utf8_lossy(&output.stdout);
            let pid_str = stdout_content.trim();
            pid_str.parse::<u32>().ok()
        } else {
            None
        }
    }
}

/// Start the stackql server with the given options
pub fn start_server(options: &StartServerOptions) -> Result<u32, String> {
    let binary_path = match get_binary_path() {
        Some(path) => path,
        _none => return Err("StackQL binary not found".to_string()),
    };

    // Check if server is already running
    if is_server_running(options.port) {
        println!(
            "{}",
            format!("Server is already running on port {}", options.port).yellow()
        );
        return Ok(get_server_pid(options.port).unwrap_or(0));
    }

    // Prepare command with all options
    let mut cmd = ProcessCommand::new(&binary_path);

    // Always include address and port
    cmd.arg("--pgsrv.address").arg(&options.host);
    cmd.arg("--pgsrv.port").arg(options.port.to_string());

    // Add optional parameters if provided
    if let Some(registry) = &options.registry {
        cmd.arg("--registry").arg(registry);
    }

    if let Some(mtls_config) = &options.mtls_config {
        cmd.arg("--mtls-config").arg(mtls_config);
    }

    if let Some(custom_auth) = &options.custom_auth_config {
        cmd.arg("--custom-auth-config").arg(custom_auth);
    }

    if let Some(log_level) = &options.log_level {
        cmd.arg("--log-level").arg(log_level);
    }

    cmd.arg("srv");

    // Setup logging
    let log_path = Path::new(DEFAULT_LOG_FILE);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("Failed to open log file: {}", e))?;

    // Start the server
    let child = cmd
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|e| format!("Failed to start server: {}", e))?;

    let pid = child.id();

    // Wait a bit for the server to start
    println!(
        "{}",
        format!("Starting stackql server with PID: {}", pid).green()
    );
    thread::sleep(Duration::from_secs(5));

    if is_server_running(options.port) {
        println!("{}", "Server started successfully".green());
        Ok(pid)
    } else {
        Err("Server failed to start properly".to_string())
    }
}

/// Stop the stackql server
pub fn stop_server(port: u16) -> Result<(), String> {
    if !is_server_running(port) {
        println!("{}", format!("No server running on port {}", port).yellow());
        return Ok(());
    }

    let pid = match get_server_pid(port) {
        Some(pid) => pid,
        _none => return Err("Could not determine server PID".to_string()),
    };

    println!(
        "{}",
        format!("Stopping stackql server with PID: {}", pid).yellow()
    );

    if cfg!(target_os = "windows") {
        ProcessCommand::new("taskkill")
            .arg("/F")
            .arg("/PID")
            .arg(pid.to_string())
            .output()
            .map_err(|e| format!("Failed to stop server: {}", e))?;
    } else {
        ProcessCommand::new("kill")
            .arg(pid.to_string())
            .output()
            .map_err(|e| format!("Failed to stop server: {}", e))?;
    }

    // Wait a bit to verify it's stopped
    thread::sleep(Duration::from_secs(1));

    if !is_server_running(port) {
        println!("{}", "Server stopped successfully".green());
        Ok(())
    } else {
        Err("Server is still running after stop attempt".to_string())
    }
}
