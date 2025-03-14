use clap::Command;
use crate::utils::display::print_unicode_box;

pub fn command() -> Command {
    Command::new("upgrade").about("Upgrade the CLI and dependencies")
}

pub fn execute() {
    print_unicode_box("⬆️ Upgrading CLI and dependencies...");
}
