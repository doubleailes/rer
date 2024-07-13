use rer_resolver::solver;
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
    let paths = vec![std::path::PathBuf::from(
        "/home/philippe.llerena/workspace/github.com/doubleailes/rer-bkp/data_set/packages",
    )];
    let solution = solver(requirements, paths);
    let elapsed = start.elapsed();
    println!("Resolve in Time: {:?}", elapsed);
    println!("{:#?}", solution);
}
