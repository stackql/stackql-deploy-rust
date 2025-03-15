use crate::utils::display::print_unicode_box;
use clap::{Arg, ArgMatches, Command};

pub fn command() -> Command {
    Command::new("teardown")
        .about("Teardown a provisioned stack")
        .arg(Arg::new("stack_dir").required(true))
        .arg(Arg::new("stack_env").required(true))
}

pub fn execute(matches: &ArgMatches) {
    let stack_dir = matches.get_one::<String>("stack_dir").unwrap();
    let stack_env = matches.get_one::<String>("stack_env").unwrap();
    print_unicode_box(&format!(
        "Tearing down stack: [{}] in environment: [{}]",
        stack_dir, stack_env
    ));
}
