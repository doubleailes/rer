use rer_resolver::{solver, LocalPackages};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let input = &args[1];
    if input.is_empty() {
        eprintln!("Package name can't be empty");
        std::process::exit(1);
    }
    let requirements = args[1..].iter().map(|x| x.as_str()).collect();
    let start = std::time::Instant::now();
    let packages = LocalPackages::build_from_json_path("data_set_private/rez_lib.json");
    println!("Load Json in Time: {:?}", start.elapsed());
    let start = std::time::Instant::now();
    let solution = solver(requirements, packages);
    let elapsed = start.elapsed();
    println!("Resolve in Time: {:?}", elapsed);
    println!("{:#?}", solution);
}
