use rer_resolver::solver;
use serde_json;
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct RezResolveBenchmark {
    request: Vec<String>,
    status: String,
    resolve_time: f32,
    resolved_packages: Option<Vec<String>>,
}

fn main() {
    let data_str_resolves =
    fs::read_to_string("data_set_private/resolves.json").expect("Unable to read file");
    let resolves: Vec<RezResolveBenchmark> = serde_json::from_str(&data_str_resolves).unwrap();
    for resolve in resolves {
        let paths = vec![std::path::PathBuf::from(
            "/home/philippe.llerena/workspace/github.com/doubleailes/rer-bkp/data_set/packages",
        )];
        let start = std::time::Instant::now();
        let solution = solver(resolve.request.iter().map(|x|x.as_str() ).collect(), paths);
        let elapsed = start.elapsed();
        println!("Result Rez {}, Rer {}", resolve.status, &solution.is_ok());
        if solution.is_ok(){
            let solution = solution.unwrap();
            println!("Solution: {} rez got {}", solution.len(), resolve.resolved_packages.unwrap().len());
        }
        println!("Resolve in Time: {:?} rez resolve it in {} ms", elapsed, resolve.resolve_time*1000.0);
    }
}
