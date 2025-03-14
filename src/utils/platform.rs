use std::env;

#[derive(Debug, PartialEq)]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
    Unknown,
}

/// Determine the current operating system
pub fn get_platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOS
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Unknown
    }
}

/// Get the appropriate binary name based on platform
pub fn get_binary_name() -> String {
    match get_platform() {
        Platform::Windows => "stackql.exe".to_string(),
        _ => "stackql".to_string(),
    }
}