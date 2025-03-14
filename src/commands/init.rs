use clap::{Arg, ArgMatches, Command};
use crate::utils::display::print_unicode_box;

pub fn command() -> Command {
    Command::new("init")
        .about("Initialize a new project")
        .arg(Arg::new("stack_name").required(true))
        .arg(Arg::new("provider").long("provider").help("Specify a provider (aws, azure, google)"))
}

pub fn execute(matches: &ArgMatches) {
    let stack_name = matches.get_one::<String>("stack_name").unwrap();
    let provider = matches.get_one::<String>("provider").map(|s| s.as_str()).unwrap_or("azure");
    
    print_unicode_box(&format!("🛠️ Initializing project [{}] with provider [{}]", stack_name, provider));
}
