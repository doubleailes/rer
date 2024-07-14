use rer_resolver::solver;
use serde_json;
use std::fs;

fn main() {
    let data_str =
        fs::read_to_string("data_set_private/requests.json").expect("Unable to read file");
    let requests: Vec<Vec<&str>> = serde_json::from_str(&data_str).unwrap();
    for requirements in requests {
        let paths = vec![std::path::PathBuf::from(
            "/home/philippe.llerena/workspace/github.com/doubleailes/rer-bkp/data_set/packages",
        )];
        let start = std::time::Instant::now();
        let solution = solver(requirements, paths);
        let elapsed = start.elapsed();
        println!("Resolve in Time: {:?}", elapsed);
        println!("{:#?}", solution);
    }
}
