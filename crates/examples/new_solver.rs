use pubgrub::{resolve, DefaultStringReporter, PubGrubError, Reporter};
use rer_resolver::resolver::RerDependencyProvider;
use rer_resolver::PackageId;
use rer_version::RerVersion;
use serde::Deserialize;
use serde_json;
use std::fs;

#[allow(dead_code)]
#[derive(Deserialize)]
struct RezResolveBenchmark {
    request: Vec<String>,
    status: String,
    resolve_time: f32,
    resolved_packages: Option<Vec<String>>,
}

fn compare_solutions(solution: &Vec<String>, resolved_rez_list: &Vec<String>) {
    for x in resolved_rez_list {
        if !solution.contains(&x) {
            eprintln!("Missing: {}", x);
        }
    }
}

fn main() {
    let paths = vec![std::path::PathBuf::from(
        "/home/philippe.llerena/workspace/github.com/doubleailes/rer-bkp/data_set/packages",
    )];
    let data_str_resolves =
        fs::read_to_string("data_set_private/resolves.json").expect("Unable to read file");
    let resolves: Vec<RezResolveBenchmark> = serde_json::from_str(&data_str_resolves).unwrap();
    for resolved in resolves {
        let start = std::time::Instant::now();
        let mut dependency_provider = RerDependencyProvider::lazy_paths(paths.clone());
        println!("Request: {:?}", resolved.request);
        dependency_provider.add_init_request(resolved.request);
        let version: RerVersion = "1.0.0".try_into().unwrap();
        let root = PackageId::Root;
        match resolve(&dependency_provider, root, version) {
            Ok(solution) => {
                let mut solution_str: Vec<String> = solution
                    .into_iter()
                    .filter(|(id, _)| matches!(id, PackageId::Base(_)))
                    .map(|(id, version)| {
                        let name = id.name().expect("Base always has a name");
                        format!("{}/{}/package.py", name, version)
                    })
                    .collect();
                solution_str.sort();
                println!("{:#?}", solution_str);
                //compare_solutions(&solution_str, &resolved.resolved_packages.unwrap());
            }
            Err(PubGrubError::NoSolution(mut derivation_tree)) => {
                derivation_tree.collapse_no_versions();
                eprintln!("{}", DefaultStringReporter::report(&derivation_tree));
            }
            Err(err) => panic!("{:?}", err),
        }
        let elapsed = start.elapsed();
        println!(
            "Resolve in Time: {:?} rez resolve it in {} ms",
            elapsed,
            resolved.resolve_time * 1000.0
        );
    }
}
