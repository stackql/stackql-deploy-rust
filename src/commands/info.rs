use clap::Command;
use crate::utils::display::print_unicode_box;

pub fn command() -> Command {
    Command::new("info").about("Display version information")
}

pub fn execute() {
    print_unicode_box("📌 CLI Version: 0.1.0\n🔍 More details coming soon...");
}
