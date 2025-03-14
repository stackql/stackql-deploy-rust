use colored::*;
use unicode_width::UnicodeWidthStr;

/// Utility function to print a Unicode-styled message box
/// that correctly handles the width of emojis and other wide characters
pub fn print_unicode_box(message: &str) {
    let border_color = "\x1b[93m"; // Yellow
    let reset_color = "\x1b[0m";
    let lines: Vec<&str> = message.split('\n').collect();
    
    // Calculate width using unicode_width to properly account for emojis
    let max_length = lines.iter()
        .map(|line| UnicodeWidthStr::width(*line))
        .max()
        .unwrap_or(0);
    
    let top_border = format!("{}┌{}┐{}", border_color, "─".repeat(max_length + 2), reset_color);
    let bottom_border = format!("{}└{}┘{}", border_color, "─".repeat(max_length + 2), reset_color);
    
    println!("{}", top_border);
    for line in lines {
        // Calculate proper padding based on the visual width
        let padding = max_length - UnicodeWidthStr::width(line);
        let padded_line = format!("│ {}{} │", line, " ".repeat(padding));
        println!("{}{}{}", border_color, padded_line, reset_color);
    }
    println!("{}", bottom_border);
}

/// Print an error message in red
pub fn print_error(message: &str) {
    eprintln!("{}", message.red());
}

/// Print a success message in green
pub fn print_success(message: &str) {
    println!("{}", message.green());
}

/// Print an info message in blue
pub fn print_info(message: &str) {
    println!("{}", message.blue());
}