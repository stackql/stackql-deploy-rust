use clap::{Arg, ArgMatches, Command};
use crate::utils::display::print_unicode_box;

pub fn command() -> Command {
    Command::new("build")
        .about("Create or update resources")
        .arg(Arg::new("stack_dir").required(true).help("Path to stack directory"))
        .arg(Arg::new("stack_env").required(true).help("Environment to deploy"))
}

pub fn execute(matches: &ArgMatches) {
    let stack_dir = matches.get_one::<String>("stack_dir").unwrap();
    let stack_env = matches.get_one::<String>("stack_env").unwrap();
    print_unicode_box(&format!("Deploying stack: [{}] to environment: [{}]", stack_dir, stack_env));
}
