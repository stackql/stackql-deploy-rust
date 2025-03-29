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

pub struct RunningServer {
    pub pid: u32,
    pub port: u16,
}

/// Check if the stackql server is running on a specific port
pub fn is_server_running(port: u16) -> bool {
    find_all_running_servers()
        .iter()
        .any(|server| server.port == port)
}

/// Find all stackql servers that are running and their ports
pub fn find_all_running_servers() -> Vec<RunningServer> {
    let mut running_servers = Vec::new();

    if cfg!(target_os = "windows") {
        let output = ProcessCommand::new("tasklist")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute tasklist"));

        let output_str = String::from_utf8_lossy(&output.stdout);

        for line in output_str.lines() {
            if line.contains("stackql") {
                if let Some(port) = extract_port_from_windows_tasklist(line) {
                    if let Some(pid) = extract_pid_from_windows_tasklist(line) {
                        running_servers.push(RunningServer { pid, port });
                    }
                }
            }
        }
    } else {
        let output = ProcessCommand::new("pgrep")
            .arg("-f")
            .arg("stackql")
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute pgrep"));

        if !output.stdout.is_empty() {
            let pids_str = String::from_utf8_lossy(&output.stdout).to_string();
            let pids = pids_str.trim().split('\n').collect::<Vec<&str>>();

            for pid_str in pids {
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    if let Some(port) = extract_port_from_ps(pid_str) {
                        running_servers.push(RunningServer { pid, port });
                    }
                }
            }
        }
    }

    running_servers
}

/// Extract port from process information on Unix-like systems using `ps`
fn extract_port_from_ps(pid: &str) -> Option<u16> {
    let ps_output = ProcessCommand::new("ps")
        .arg("-p")
        .arg(pid)
        .arg("-o")
        .arg("args")
        .output()
        .ok()?;

    let ps_str = String::from_utf8_lossy(&ps_output.stdout);

    let patterns = [
        "--pgsrv.port=",
        "--pgsrv.port ",
        "pgsrv.port=",
        "pgsrv.port ",
    ];
    for pattern in patterns.iter() {
        if let Some(start_index) = ps_str.find(pattern) {
            let port_start = start_index + pattern.len();
            let port_end = ps_str[port_start..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim();

            if let Ok(port) = port_end.parse::<u16>() {
                return Some(port);
            }
        }
    }

    None
}

/// Extract PID from process information on Windows
fn extract_pid_from_windows_tasklist(line: &str) -> Option<u32> {
    line.split_whitespace()
        .filter_map(|s| s.parse::<u32>().ok())
        .next()
}

/// Extract port from process information on Windows
fn extract_port_from_windows_tasklist(line: &str) -> Option<u16> {
    if let Some(port_str) = line.split_whitespace().find(|&s| s.parse::<u16>().is_ok()) {
        port_str.parse().ok()
    } else {
        None
    }
}

/// Get the PID of the running stackql server on a specific port
pub fn get_server_pid(port: u16) -> Option<u32> {
    let patterns = [
        format!("stackql.*--pgsrv.port={}", port),
        format!("stackql.*--pgsrv.port {}", port),
        format!("stackql.*pgsrv.port={}", port),
        format!("stackql.*pgsrv.port {}", port),
    ];

    for pattern in &patterns {
        let output = ProcessCommand::new("pgrep")
            .arg("-f")
            .arg(pattern)
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

    None
}

/// Start the stackql server with the given options
pub fn start_server(options: &StartServerOptions) -> Result<u32, String> {
    let binary_path = match get_binary_path() {
        Some(path) => path,
        _none => return Err("StackQL binary not found".to_string()),
    };

    if is_server_running(options.port) {
        println!(
            "{}",
            format!("Server is already running on port {}", options.port).yellow()
        );
        return Ok(get_server_pid(options.port).unwrap_or(0));
    }

    let mut cmd = ProcessCommand::new(&binary_path);
    cmd.arg("--pgsrv.address").arg(&options.host);
    cmd.arg("--pgsrv.port").arg(options.port.to_string());

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

    let log_path = Path::new(DEFAULT_LOG_FILE);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("Failed to open log file: {}", e))?;

    let child = cmd
        .stdout(Stdio::from(log_file.try_clone().unwrap()))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|e| format!("Failed to start server: {}", e))?;

    let pid = child.id();
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

    Ok(())
}
