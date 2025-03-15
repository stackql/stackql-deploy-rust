use std::error::Error;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum AppError {
    BinaryNotFound,
    CommandFailed(String),
    IoError(std::io::Error),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BinaryNotFound => write!(f, "The stackql binary was not found"),
            Self::CommandFailed(msg) => write!(f, "Command failed: {}", msg),
            Self::IoError(err) => write!(f, "IO error: {}", err),
        }
    }
}

impl Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        Self::IoError(error)
    }
}

// New helper function
pub fn get_binary_path_with_error() -> Result<PathBuf, AppError> {
    crate::utils::binary::get_binary_path().ok_or(AppError::BinaryNotFound)
}
