use clap::{Arg, ArgMatches, Command};
use crate::utils::display::print_unicode_box;

pub fn command() -> Command {
    Command::new("test")
        .about("Run test queries for the stack")
        .arg(Arg::new("stack_dir").required(true))
        .arg(Arg::new("stack_env").required(true))
}

pub fn execute(matches: &ArgMatches) {
    let stack_dir = matches.get_one::<String>("stack_dir").unwrap();
    let stack_env = matches.get_one::<String>("stack_env").unwrap();
    print_unicode_box(&format!("Testing stack: [{}] in environment: [{}]", stack_dir, stack_env));
}
