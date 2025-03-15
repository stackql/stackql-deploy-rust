use crate::utils::display::print_unicode_box;
use clap::Command;

pub fn command() -> Command {
    Command::new("plan").about("Plan infrastructure changes (coming soon)")
}

pub fn execute() {
    print_unicode_box("🔮 Infrastructure planning (coming soon)...");
    println!("The 'plan' feature is coming soon!");
}
