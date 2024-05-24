use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
pub struct App {
    #[clap(subcommand)]
    pub command: Command
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Env {
        #[arg(num_args(0..))]
        pkg_request: Vec<String>
    }
}