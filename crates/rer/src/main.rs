mod cli;

use clap::Parser;

use cli::{App, Command};
use env::hello_rer_env;

fn main() {
    println!("Hello, RER!");

    let app = App::parse();
    match app.command {
        Command::Env { pkg_request } => {
            hello_rer_env(pkg_request);
        }
    }
}
