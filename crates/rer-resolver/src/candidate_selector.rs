use pubgrub::range::Range;
use pubgrub::version::Version;
use rer_version::RerVersion;

pub struct CandidateList(Vec<RerVersion>);

impl CandidateList {
    pub fn new(mut candidat_list: Vec<RerVersion>) -> Self {
        candidat_list.sort();
        CandidateList(candidat_list)
    }
    pub fn sort(&mut self) {
        self.0.sort();
    }
    pub fn from_vec_str(v: Vec<&str>) -> Self {
        let mut list: Vec<RerVersion> = v
            .into_iter()
            .map(|x| x.try_into().expect("can't convert"))
            .collect();
        list.sort();
        CandidateList(list)
    }
    pub fn find_candidate(
        &self,
        range: &Range<RerVersion>,
        strategy_mode: ResolutionMode,
    ) -> Option<RerVersion> {
        match strategy_mode {
            ResolutionMode::Highest => self.0.iter().rev().find(|&x| range.contains(x)).cloned(),
            ResolutionMode::Lowest => self.0.iter().find(|&x| range.contains(x)).cloned(),
        }
    }
    pub fn find_candidates(&self, range: &Range<RerVersion>) -> Vec<&RerVersion> {
        self.0.iter().filter(|&x| range.contains(x)).collect()
    }
}

#[test]
fn test_candidate_list() {
    let list = CandidateList::from_vec_str(vec!["1.0.0", "1.1.0", "1.2.0"]);
    let v1: RerVersion = "1.0.0".try_into().unwrap();
    let v2: RerVersion = "1.2.0".try_into().unwrap();
    let range = Range::between(v1, v2);
    let v3: RerVersion = "1.1.0".try_into().unwrap();
    assert_eq!(
        list.find_candidate(&range, ResolutionMode::Highest),
        Some(v3)
    );
    let v3: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(
        list.find_candidate(&range, ResolutionMode::Lowest),
        Some(v3)
    );
}

#[test]
fn test_candidates_list() {
    let list = CandidateList::from_vec_str(vec!["1.0.0", "1.1.0", "1.1.1", "1.2.0"]);
    let start: RerVersion = "1.0.0".try_into().unwrap();
    let end: RerVersion = "1.2.0".try_into().unwrap();
    let range = Range::between(start, end);
    let v1: RerVersion = "1.0.0".try_into().unwrap();
    let v3: RerVersion = "1.1.0".try_into().unwrap();
    let v4: RerVersion = "1.1.1".try_into().unwrap();
    let mut results = vec![&v1, &v3, &v4];
    results.sort();
    assert_eq!(list.find_candidates(&range), results);
    let list =
        CandidateList::from_vec_str(vec!["4.8.6.m1", "4.8.6.m2", "5.12.6", "5.6.1", "4.8.6.m3"]);
    let v1: RerVersion = "4.8.6".try_into().unwrap();
    let range = Range::between(v1.clone(), v1.bump());
    let v2: RerVersion = "4.8.6.m3".try_into().unwrap();
    assert_eq!(
        list.find_candidate(&range, ResolutionMode::Highest),
        Some(v2)
    );
}
#[derive(Debug, Default)]
pub enum ResolutionMode {
    /// Resolve the highest compatible version of each package.
    #[default]
    Highest,
    /// Resolve the lowest compatible version of each package.
    Lowest,
}
