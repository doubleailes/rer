// REZ-ENV doc : https://rez.readthedocs.io/en/stable/commands/rez-env.html
// REZ-ENV src : https://github.com/AcademySoftwareFoundation/rez/blob/main/src/rez/cli/env.py
// TODO : 
// - [ ] config
// - [ ] pkg listing
// - [ ] pkg parsing
// - [ ] resolver
// - [ ] context builder
// - [ ] shell 

use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(num_args(0..))]
    pkg_request: Vec<String>
}

fn main() {
    println!("Hello, RER-ENV !");

    let args = Args::parse();

    println!("Resolving : {:#?}", args.pkg_request);
}
