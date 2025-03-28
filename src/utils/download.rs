// use crate::error::AppError;
// use crate::utils::display::print_info;
// use crate::utils::platform::{get_platform, Platform};
// use indicatif::{ProgressBar, ProgressStyle};
// use reqwest::blocking::Client;
// use std::fs::{self, File};
// use std::io::{self, Write};
// use std::path::{Path, PathBuf};
// use std::process::Command;
// use zip::ZipArchive;

// pub fn get_download_url() -> Result<String, AppError> {
//     match get_platform() {
//         Platform::Linux => Ok("https://releases.stackql.io/stackql/latest/stackql_linux_amd64.zip".to_string()),
//         Platform::Windows => Ok("https://releases.stackql.io/stackql/latest/stackql_windows_amd64.zip".to_string()),
//         Platform::MacOS => Ok("https://storage.googleapis.com/stackql-public-releases/latest/stackql_darwin_multiarch.pkg".to_string()),
//         Platform::Unknown => Err(AppError::CommandFailed("Unsupported OS".to_string())),
//     }
// }

// pub fn download_binary() -> Result<PathBuf, AppError> {
//     let download_url = get_download_url()?;
//     let current_dir = std::env::current_dir().map_err(AppError::IoError)?;
//     let binary_name = crate::utils::platform::get_binary_name();
//     let archive_name = Path::new(&download_url)
//         .file_name()
//         .ok_or_else(|| AppError::CommandFailed("Invalid URL".to_string()))?
//         .to_string_lossy()
//         .to_string();
//     let archive_path = current_dir.join(&archive_name);

//     // Download the file with progress bar
//     print_info(&format!("Downloading from {}", download_url));
//     let client = Client::new();
//     let response = client
//         .get(&download_url)
//         .send()
//         .map_err(|e| AppError::CommandFailed(format!("Failed to download: {}", e)))?;

//     let total_size = response.content_length().unwrap_or(0);
//     let progress_bar = ProgressBar::new(total_size);
//     progress_bar.set_style(
//         ProgressStyle::default_bar()
//             .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
//             .unwrap()
//             .progress_chars("#>-"));

//     let mut file = File::create(&archive_path).map_err(AppError::IoError)?;
//     let mut _downloaded: u64 = 0;
//     let stream = response
//         .bytes()
//         .map_err(|e| AppError::CommandFailed(format!("Failed to read response: {}", e)))?;

//     file.write_all(&stream).map_err(AppError::IoError)?;
//     progress_bar.finish_with_message("Download complete");

//     // Extract the file based on platform
//     print_info("Extracting the binary...");
//     let binary_path = extract_binary(&archive_path, &current_dir, &binary_name)?;

//     // Clean up the archive
//     fs::remove_file(&archive_path).ok();

//     // Set executable permissions on Unix-like systems
//     if get_platform() != Platform::Windows {
//         Command::new("chmod")
//             .arg("+x")
//             .arg(&binary_path)
//             .output()
//             .map_err(|e| {
//                 AppError::CommandFailed(format!("Failed to set executable permission: {}", e))
//             })?;
//     }

//     print_info(&format!(
//         "StackQL executable successfully installed at: {}",
//         binary_path.display()
//     ));
//     Ok(binary_path)
// }

// fn extract_binary(
//     archive_path: &Path,
//     dest_dir: &Path,
//     binary_name: &str,
// ) -> Result<PathBuf, AppError> {
//     let binary_path = dest_dir.join(binary_name);

//     match get_platform() {
//         Platform::MacOS => {
//             // For macOS, we need to use pkgutil
//             let unpacked_dir = dest_dir.join("stackql_unpacked");
//             if unpacked_dir.exists() {
//                 fs::remove_dir_all(&unpacked_dir).map_err(AppError::IoError)?;
//             }
//             fs::create_dir_all(&unpacked_dir).map_err(AppError::IoError)?;

//             Command::new("pkgutil")
//                 .arg("--expand-full")
//                 .arg(archive_path)
//                 .arg(&unpacked_dir)
//                 .output()
//                 .map_err(|e| AppError::CommandFailed(format!("Failed to extract pkg: {}", e)))?;

//             // Find and copy the binary
//             // This might need adjustment based on the actual structure of the pkg
//             // Typically you'd need to look for the binary in the expanded package

//             // Example (adjust paths as needed):
//             let extracted_binary = unpacked_dir
//                 .join("payload")
//                 .join("usr")
//                 .join("local")
//                 .join("bin")
//                 .join("stackql");
//             fs::copy(extracted_binary, &binary_path).map_err(AppError::IoError)?;

//             // Clean up
//             fs::remove_dir_all(unpacked_dir).ok();
//         }
//         _ => {
//             // For Windows and Linux, we use the zip file
//             let file = File::open(archive_path).map_err(AppError::IoError)?;
//             let mut archive = ZipArchive::new(file).map_err(|e| {
//                 AppError::CommandFailed(format!("Failed to open zip archive: {}", e))
//             })?;

//             for i in 0..archive.len() {
//                 let mut file = archive.by_index(i).map_err(|e| {
//                     AppError::CommandFailed(format!("Failed to extract file: {}", e))
//                 })?;

//                 let outpath = match file.enclosed_name() {
//                     Some(path) => dest_dir.join(path),
//                     None => continue,
//                 };

//                 if file.name().ends_with('/') {
//                     fs::create_dir_all(&outpath).map_err(AppError::IoError)?;
//                 } else {
//                     let mut outfile = File::create(&outpath).map_err(AppError::IoError)?;
//                     io::copy(&mut file, &mut outfile).map_err(AppError::IoError)?;
//                 }
//             }

//             // Check if we need to rename the binary on Windows
//             if get_platform() == Platform::Windows {
//                 let potential_binary = dest_dir.join("stackql");
//                 if potential_binary.exists() && !binary_path.exists() {
//                     fs::rename(potential_binary, &binary_path).map_err(AppError::IoError)?;
//                 }
//             }
//         }
//     }

//     if !binary_path.exists() {
//         return Err(AppError::CommandFailed(format!(
//             "Binary {} not found after extraction",
//             binary_name
//         )));
//     }

//     Ok(binary_path)
// }

use crate::app::STACKQL_DOWNLOAD_URL;
use crate::error::AppError;
use crate::utils::display::print_info;
use crate::utils::platform::{get_platform, Platform};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::ZipArchive;

pub fn get_download_url() -> Result<String, AppError> {
    Ok(STACKQL_DOWNLOAD_URL.to_string())
}

pub fn download_binary() -> Result<PathBuf, AppError> {
    let download_url = get_download_url()?;
    let current_dir = std::env::current_dir().map_err(AppError::IoError)?;
    let binary_name = crate::utils::platform::get_binary_name();
    let archive_name = Path::new(&download_url)
        .file_name()
        .ok_or_else(|| AppError::CommandFailed("Invalid URL".to_string()))?
        .to_string_lossy()
        .to_string();
    let archive_path = current_dir.join(&archive_name);

    // Download the file with progress bar
    print_info(&format!("Downloading from {}", download_url));
    let client = Client::new();
    let response = client
        .get(&download_url)
        .send()
        .map_err(|e| AppError::CommandFailed(format!("Failed to download: {}", e)))?;

    let total_size = response.content_length().unwrap_or(0);
    let progress_bar = ProgressBar::new(total_size);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

    let mut file = File::create(&archive_path).map_err(AppError::IoError)?;
    let mut _downloaded: u64 = 0;
    let stream = response
        .bytes()
        .map_err(|e| AppError::CommandFailed(format!("Failed to read response: {}", e)))?;

    file.write_all(&stream).map_err(AppError::IoError)?;
    progress_bar.finish_with_message("Download complete");

    // Extract the file based on platform
    print_info("Extracting the binary...");
    let binary_path = extract_binary(&archive_path, &current_dir, &binary_name)?;

    // Clean up the archive
    fs::remove_file(&archive_path).ok();

    // Set executable permissions on Unix-like systems
    if get_platform() != Platform::Windows {
        Command::new("chmod")
            .arg("+x")
            .arg(&binary_path)
            .output()
            .map_err(|e| {
                AppError::CommandFailed(format!("Failed to set executable permission: {}", e))
            })?;
    }

    print_info(&format!(
        "StackQL executable successfully installed at: {}",
        binary_path.display()
    ));
    Ok(binary_path)
}

fn extract_binary(
    archive_path: &Path,
    dest_dir: &Path,
    binary_name: &str,
) -> Result<PathBuf, AppError> {
    let binary_path = dest_dir.join(binary_name);

    match get_platform() {
        Platform::MacOS => {
            // For macOS, we need to use pkgutil
            let unpacked_dir = dest_dir.join("stackql_unpacked");
            if unpacked_dir.exists() {
                fs::remove_dir_all(&unpacked_dir).map_err(AppError::IoError)?;
            }
            fs::create_dir_all(&unpacked_dir).map_err(AppError::IoError)?;

            Command::new("pkgutil")
                .arg("--expand-full")
                .arg(archive_path)
                .arg(&unpacked_dir)
                .output()
                .map_err(|e| AppError::CommandFailed(format!("Failed to extract pkg: {}", e)))?;

            // Find and copy the binary
            // This might need adjustment based on the actual structure of the pkg
            // Typically you'd need to look for the binary in the expanded package

            // Example (adjust paths as needed):
            let extracted_binary = unpacked_dir
                .join("payload")
                .join("usr")
                .join("local")
                .join("bin")
                .join("stackql");
            fs::copy(extracted_binary, &binary_path).map_err(AppError::IoError)?;

            // Clean up
            fs::remove_dir_all(unpacked_dir).ok();
        }
        _ => {
            // For Windows and Linux, we use the zip file
            let file = File::open(archive_path).map_err(AppError::IoError)?;
            let mut archive = ZipArchive::new(file).map_err(|e| {
                AppError::CommandFailed(format!("Failed to open zip archive: {}", e))
            })?;

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).map_err(|e| {
                    AppError::CommandFailed(format!("Failed to extract file: {}", e))
                })?;

                let outpath = match file.enclosed_name() {
                    Some(path) => dest_dir.join(path),
                    None => continue,
                };

                if file.name().ends_with('/') {
                    fs::create_dir_all(&outpath).map_err(AppError::IoError)?;
                } else {
                    let mut outfile = File::create(&outpath).map_err(AppError::IoError)?;
                    io::copy(&mut file, &mut outfile).map_err(AppError::IoError)?;
                }
            }

            // Check if we need to rename the binary on Windows
            if get_platform() == Platform::Windows {
                let potential_binary = dest_dir.join("stackql");
                if potential_binary.exists() && !binary_path.exists() {
                    fs::rename(potential_binary, &binary_path).map_err(AppError::IoError)?;
                }
            }
        }
    }

    if !binary_path.exists() {
        return Err(AppError::CommandFailed(format!(
            "Binary {} not found after extraction",
            binary_name
        )));
    }

    Ok(binary_path)
}
