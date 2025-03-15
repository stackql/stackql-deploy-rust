use std::env;
use std::path::PathBuf;
use std::process::Command;

// /// Check if the stackql binary exists in the current directory
// pub fn binary_exists_in_current_dir() -> bool {
//     if let Ok(current_dir) = env::current_dir() {
//         let binary_name = super::platform::get_binary_name();
//         let binary_path = current_dir.join(&binary_name);
//         binary_path.exists() && binary_path.is_file()
//     } else {
//         false
//     }
// }

/// Check if the stackql binary exists in PATH
pub fn binary_exists_in_path() -> bool {
    let binary_name = super::platform::get_binary_name();
    let status = if super::platform::get_platform() == super::platform::Platform::Windows {
        Command::new("where").arg(&binary_name).status()
    } else {
        Command::new("which").arg(&binary_name).status()
    };

    status.map(|s| s.success()).unwrap_or(false)
}

/// Get the full path to the stackql binary
pub fn get_binary_path() -> Option<PathBuf> {
    let binary_name = super::platform::get_binary_name();

    // First check current directory
    if let Ok(current_dir) = env::current_dir() {
        let binary_path = current_dir.join(&binary_name);
        if binary_path.exists() && binary_path.is_file() {
            return Some(binary_path);
        }
    }

    // Then check PATH
    if binary_exists_in_path() {
        if let Ok(paths) = env::var("PATH") {
            for path in env::split_paths(&paths) {
                let full_path = path.join(&binary_name);
                if full_path.exists() && full_path.is_file() {
                    return Some(full_path);
                }
            }
        }
    }

    None
}
