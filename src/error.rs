use std::fmt;
use std::error::Error;

#[derive(Debug)]
pub enum AppError {
    BinaryNotFound,
    CommandFailed(String),
    IoError(std::io::Error),
    // Add more error types as needed
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