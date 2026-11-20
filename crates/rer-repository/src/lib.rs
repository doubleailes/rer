//! rez's `filesystem` package repository, in pure Rust.
//!
//! Ports the discovery half of `rezplugins/package_repository/filesystem.py`
//! (family lookup, version directories, `.ignore<ver>` / `.building<ver>`
//! markers, definition-file selection) and the post-load steps of
//! `FileSystemPackageResource._load` (`format_version`, old-format metadata),
//! on top of [`rer_package::definition`] for the file contents.
//!
//! The contract is the same as `rer-package`'s: either reproduce what rez
//! would produce, or say so. Two granularities:
//!
//! - a whole **family** is handed back ([`FamilyLoad::NeedsPython`]) when
//!   its on-disk layout is something rer doesn't model (combined
//!   `<name>.py` families, unversioned packages, version directories rez
//!   can't parse, versions that compare equal);
//! - a single **package** is handed back ([`PackageOutcome::NeedsPython`])
//!   when its definition file is dynamic, missing, or otherwise not modelled.
//!
//! Caching mirrors rez's in-process behaviour: listings and loaded
//! definitions are cached per repository with no invalidation until
//! [`FsRepository::clear_caches`], like rez's `lru_cache`s and resource pool.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rayon::prelude::*;
use regex::Regex;
use rer_package::definition::{
    load_package_py, load_package_yaml, LoadOutcome, NeedsPython, NeedsPythonReason,
    PackageDefinition, Value,
};
use rer_version::{RerVersion, VersionRange};

/// Settings of rez's filesystem repository plugin that change what loading
/// produces. Defaults are rez's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsSettings {
    /// `plugins.package_repository.filesystem.package_filenames`.
    pub package_filenames: Vec<String>,
    /// `allow_unversioned_packages`.
    pub allow_unversioned_packages: bool,
    /// `plugins.package_repository.filesystem.check_package_definition_files`.
    pub check_package_definition_files: bool,
    /// Repository constructed with `disable_pkg_ignore=True`.
    pub disable_pkg_ignore: bool,
}

impl Default for FsSettings {
    fn default() -> Self {
        FsSettings {
            package_filenames: vec!["package".to_string()],
            allow_unversioned_packages: true,
            check_package_definition_files: false,
            disable_pkg_ignore: false,
        }
    }
}

/// Definition file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileFormat {
    Py,
    Yaml,
}

/// The result of loading one package (one version directory).
#[derive(Clone, Debug)]
pub enum PackageOutcome {
    /// `FileSystemPackageResource._data`, exactly.
    Static(Arc<PackageDefinition>),
    NeedsPython(NeedsPython),
}

/// One package of a family, as rez's `iter_packages` would yield it.
#[derive(Clone, Debug)]
pub struct LoadedPackage {
    /// The family directory name (rez's `pkg.name`).
    pub name: String,
    /// The version directory name (rez's `str(pkg.version)`).
    pub version: String,
    /// The definition file, when one was found.
    pub filepath: Option<PathBuf>,
    pub outcome: PackageOutcome,
}

/// The result of loading a family across all repositories.
#[derive(Clone, Debug)]
pub enum FamilyLoad {
    /// Packages in rez's `iter_packages` order of discovery (first
    /// repository wins on equal versions), filtered by the range.
    Packages(Vec<LoadedPackage>),
    /// rez must enumerate this family itself; the reason is diagnostic.
    NeedsPython(String),
}

/// What one repository holds for a family name.
#[derive(Clone, Debug)]
enum FamilyListing {
    Absent,
    /// Version directory names (unordered, like rez's `list(set)`).
    Versions(Vec<String>),
    NeedsPython(String),
}

/// One `packages_path` entry.
pub struct FsRepository {
    location: PathBuf,
    settings: FsSettings,
    families: Mutex<HashMap<String, FamilyListing>>,
    definitions: Mutex<HashMap<PathBuf, PackageOutcome>>,
}

fn package_name_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[a-zA-Z_0-9](\.?[a-zA-Z0-9_]+)*\z").unwrap())
}

/// `rez.utils.formatting.is_valid_package_name`.
pub fn is_valid_package_name(name: &str) -> bool {
    package_name_regex().is_match(name) && name != "__pycache__"
}

/// The tokens of a rez `Version`, or `None` where `Version(s)` raises.
///
/// Two versions are equal in rez iff their token strings are equal
/// (separators are ignored, `_SubToken` compares strings exactly).
pub fn rez_version_tokens(s: &str) -> Option<Vec<&str>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    let is_tok = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut tokens = Vec::new();
    let mut seps_ok = true;
    let mut i = 0;
    let bytes: Vec<(usize, char)> = s.char_indices().collect();
    // seps[0] and seps[-1] must be empty; inner seps exactly one char.
    let mut expect_token = true;
    let mut sep_len = 0;
    while i < bytes.len() {
        let (start, c) = bytes[i];
        if is_tok(c) {
            let mut j = i;
            while j < bytes.len() && is_tok(bytes[j].1) {
                j += 1;
            }
            let end = bytes.get(j).map(|b| b.0).unwrap_or(s.len());
            if !expect_token && sep_len != 1 {
                seps_ok = false;
            }
            tokens.push(&s[start..end]);
            expect_token = false;
            sep_len = 0;
            i = j;
        } else {
            if tokens.is_empty() {
                seps_ok = false; // leading separator
            }
            sep_len += 1;
            i += 1;
        }
    }
    if sep_len > 0 || !seps_ok || tokens.is_empty() {
        return None;
    }
    Some(tokens)
}

fn is_file(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

fn is_dir(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

/// Whether rez considers the filesystem case-sensitive
/// (`platform_.has_case_sensitive_filesystem`): Linux only.
const CASE_SENSITIVE_FS: bool = cfg!(target_os = "linux");

/// `rez.utils.filesystem.canonical_path`: realpath + normpath, lowercased on
/// case-insensitive platforms.
fn canonical_path(p: &Path) -> PathBuf {
    let real = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    #[cfg(windows)]
    let real = {
        // Strip the verbatim prefix std adds (`\\?\C:\...`).
        let s = real.to_string_lossy();
        PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
    };
    if CASE_SENSITIVE_FS {
        real
    } else {
        PathBuf::from(real.to_string_lossy().to_lowercase())
    }
}

impl FsRepository {
    pub fn new(location: impl AsRef<Path>, settings: FsSettings) -> Self {
        FsRepository {
            location: canonical_path(location.as_ref()),
            settings,
            families: Mutex::new(HashMap::new()),
            definitions: Mutex::new(HashMap::new()),
        }
    }

    /// The canonical location (rez's `repo.location`).
    pub fn location(&self) -> &Path {
        &self.location
    }

    /// Drop every cached listing and definition
    /// (`package_repository_manager.clear_caches()`).
    pub fn clear_caches(&self) {
        self.families.lock().unwrap().clear();
        self.definitions.lock().unwrap().clear();
    }

    /// `_get_file(path, package_filename)`: first `<name>.py` / `<name>.yaml`
    /// that is a file, in `package_filenames` order.
    fn get_file(&self, dir: &Path, only: Option<&str>) -> Option<(PathBuf, FileFormat)> {
        let names: Vec<&str> = match only {
            Some(n) => vec![n],
            None => self
                .settings
                .package_filenames
                .iter()
                .map(String::as_str)
                .collect(),
        };
        for name in names {
            for (ext, fmt) in [("py", FileFormat::Py), ("yaml", FileFormat::Yaml)] {
                let p = dir.join(format!("{name}.{ext}"));
                if is_file(&p) {
                    return Some((p, fmt));
                }
            }
        }
        None
    }

    fn listdir_contains(&self, name: &str) -> bool {
        fs::read_dir(&self.location)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.file_name().to_string_lossy() == name)
            })
            .unwrap_or(false)
    }

    fn family_listing(&self, name: &str) -> FamilyListing {
        if let Some(l) = self.families.lock().unwrap().get(name) {
            return l.clone();
        }
        let listing = self.compute_family_listing(name);
        self.families
            .lock()
            .unwrap()
            .insert(name.to_string(), listing.clone());
        listing
    }

    /// `_get_family(name)` + `FileSystemPackageFamilyResource.iter_packages`
    /// up to (not including) loading the packages.
    fn compute_family_listing(&self, name: &str) -> FamilyListing {
        if !is_valid_package_name(name) {
            return FamilyListing::NeedsPython(format!("invalid package name {name:?}"));
        }
        let family_dir = self.location.join(name);
        if is_dir(&family_dir) {
            if !CASE_SENSITIVE_FS && !self.listdir_contains(name) {
                return FamilyListing::Absent;
            }
        } else {
            return match self.get_file(&self.location, Some(name)) {
                Some(_) => FamilyListing::NeedsPython(format!("combined family file for {name:?}")),
                None => FamilyListing::Absent,
            };
        }

        // Unversioned package: a definition file directly in the family dir.
        if self.settings.allow_unversioned_packages && self.get_file(&family_dir, None).is_some() {
            return FamilyListing::NeedsPython(format!("unversioned package {name:?}"));
        }

        let versions = match self.version_dirs(&family_dir) {
            Ok(v) => v,
            Err(e) => return FamilyListing::NeedsPython(e),
        };
        let mut versions: Vec<String> = if self.settings.check_package_definition_files {
            versions
                .into_iter()
                .filter(|v| self.get_file(&family_dir.join(v), None).is_some())
                .collect()
        } else {
            versions
        };

        // rez computes Version(dir) for every listed dir (dedup key), so an
        // unparseable one makes the whole iteration raise; equal versions
        // are deduplicated in set order, which rer can't reproduce.
        let mut seen: HashSet<Vec<&str>> = HashSet::new();
        for v in &versions {
            match rez_version_tokens(v) {
                None => {
                    return FamilyListing::NeedsPython(format!(
                        "invalid version dir {v:?} in {name:?}"
                    ))
                }
                Some(toks) => {
                    if !seen.insert(toks) {
                        return FamilyListing::NeedsPython(format!(
                            "version dirs comparing equal in {name:?}"
                        ));
                    }
                }
            }
        }
        versions.sort();
        FamilyListing::Versions(versions)
    }

    /// `_get_version_dirs(root)`.
    fn version_dirs(&self, root: &Path) -> Result<Vec<String>, String> {
        let entries: Vec<String> = fs::read_dir(root)
            .map_err(|e| format!("listdir {}: {e}", root.display()))?
            .map(|e| {
                e.map_err(|e| e.to_string()).and_then(|e| {
                    e.file_name()
                        .into_string()
                        .map_err(|_| "non-UTF-8 name".into())
                })
            })
            .collect::<Result<_, String>>()?;
        let ignored = |name: &str| {
            !self.settings.disable_pkg_ignore && is_file(&root.join(format!(".ignore{name}")))
        };

        if self.settings.check_package_definition_files {
            return Ok(entries
                .into_iter()
                .filter(|name| {
                    !name.starts_with('.') && {
                        let p = root.join(name);
                        is_dir(&p) && !ignored(name) && self.get_file(&p, Some("package")).is_some()
                    }
                })
                .collect());
        }

        let mut dirs: HashSet<String> = HashSet::new();
        let mut building: HashSet<String> = HashSet::new();
        for name in entries {
            if name.starts_with('.') {
                if !name.starts_with(".building") {
                    continue;
                }
                building.insert(name[".building".len()..].to_string());
                // No `continue` here in rez: a `.building*` *directory* is
                // itself added as a version (and then fails Version()).
            }
            if is_dir(&root.join(&name)) && !ignored(&name) {
                dirs.insert(name);
            }
        }
        for name in building {
            if dirs.contains(&name) && self.get_file(&root.join(&name), Some("package")).is_none() {
                dirs.remove(&name);
            }
        }
        Ok(dirs.into_iter().collect())
    }

    /// `FileSystemPackageResource._data` for the package in `dir`.
    fn load_package(&self, dir: &Path) -> (Option<PathBuf>, PackageOutcome) {
        let Some((filepath, fmt)) = self.get_file(dir, None) else {
            return (
                None,
                PackageOutcome::NeedsPython(np(
                    NeedsPythonReason::RezError,
                    "missing package definition file",
                )),
            );
        };
        if let Some(o) = self.definitions.lock().unwrap().get(&filepath) {
            return (Some(filepath), o.clone());
        }
        let outcome = load_definition_file(&filepath, fmt, dir);
        self.definitions
            .lock()
            .unwrap()
            .insert(filepath.clone(), outcome.clone());
        (Some(filepath), outcome)
    }
}

fn np(reason: NeedsPythonReason, detail: &str) -> NeedsPython {
    NeedsPython {
        reason,
        line: None,
        detail: detail.to_string(),
    }
}

/// `load_from_file` + `check_format_version` + old-format merge check.
pub fn load_definition_file(
    filepath: &Path,
    fmt: FileFormat,
    package_dir: &Path,
) -> PackageOutcome {
    let bytes = match fs::read(filepath) {
        Ok(b) => b,
        Err(e) => {
            return PackageOutcome::NeedsPython(np(
                NeedsPythonReason::RezError,
                &format!("read {}: {e}", filepath.display()),
            ))
        }
    };
    let outcome = match fmt {
        FileFormat::Py => load_package_py(&bytes),
        FileFormat::Yaml => load_package_yaml(&bytes),
    };
    let mut def = match outcome {
        LoadOutcome::Static(d) => d,
        LoadOutcome::NeedsPython(n) => return PackageOutcome::NeedsPython(n),
    };
    // check_format_version: pop; a value above 2 prints a warning in rez.
    if let Some(fv) = def.remove("format_version") {
        let n = match fv {
            Value::None => None,
            Value::Int(i) => Some(i),
            Value::Bool(b) => Some(b as i64),
            Value::Str(ref s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                s.parse().ok()
            }
            _ => {
                return PackageOutcome::NeedsPython(np(
                    NeedsPythonReason::UnsupportedLiteral,
                    "format_version of an unmodelled type",
                ))
            }
        };
        if n.is_some_and(|n| n > 2) {
            return PackageOutcome::NeedsPython(np(
                NeedsPythonReason::RezError,
                "format_version > 2 (rez prints a warning)",
            ));
        }
    }
    // Old formats: rez merges release.yaml / .metadata when there's no timestamp.
    if def.get("timestamp").is_none()
        && (is_file(&package_dir.join("release.yaml")) || is_dir(&package_dir.join(".metadata")))
    {
        return PackageOutcome::NeedsPython(np(
            NeedsPythonReason::UnsupportedStatement,
            "old-format release metadata to merge",
        ));
    }
    PackageOutcome::Static(Arc::new(def))
}

/// The solver-relevant fields of a loaded definition: `requires` and
/// `variants` as raw strings, validated the way rez's package schema would
/// accept them (exact `list` of `str`; `@late` values need Python).
pub fn solver_fields(def: &PackageDefinition) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    fn str_list(v: &Value, what: &str) -> Result<Vec<String>, String> {
        match v {
            Value::List(items) => items
                .iter()
                .map(|x| match x {
                    Value::Str(s) => Ok(s.clone()),
                    _ => Err(format!("{what}: non-string item")),
                })
                .collect(),
            Value::Source(_) => Err(format!("{what}: late-bound")),
            _ => Err(format!("{what}: not a list")),
        }
    }
    let requires = match def.get("requires") {
        None => Vec::new(),
        Some(v) => str_list(v, "requires")?,
    };
    let variants = match def.get("variants") {
        None => Vec::new(),
        Some(Value::List(vs)) => vs
            .iter()
            .map(|v| str_list(v, "variants"))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("variants: not a list".into()),
    };
    Ok((requires, variants))
}

/// The configured `packages_path`, in order.
pub struct Repositories {
    repos: Vec<FsRepository>,
}

impl Repositories {
    pub fn new(paths: &[impl AsRef<Path>], settings: FsSettings) -> Self {
        Repositories {
            repos: paths
                .iter()
                .map(|p| FsRepository::new(p, settings.clone()))
                .collect(),
        }
    }

    pub fn repositories(&self) -> &[FsRepository] {
        &self.repos
    }

    pub fn clear_caches(&self) {
        for r in &self.repos {
            r.clear_caches();
        }
    }

    /// `rez.packages.iter_packages(name, range_, paths)`, loading each
    /// yielded package's definition (in parallel).
    pub fn load_family(&self, name: &str, range: Option<&VersionRange>) -> FamilyLoad {
        match self.select_family(name, range) {
            Ok(sel) => FamilyLoad::Packages(self.load_selection(name, sel)),
            Err(why) => FamilyLoad::NeedsPython(why),
        }
    }

    /// The listing half of [`load_family`](Self::load_family): which
    /// `(repository index, version dir)` pairs rez would yield, in order.
    /// `Err` means the family must be enumerated by rez.
    pub fn select_family(
        &self,
        name: &str,
        range: Option<&VersionRange>,
    ) -> Result<Selection, String> {
        // Any repository needing Python hands the whole family to rez.
        let mut listings = Vec::with_capacity(self.repos.len());
        for (idx, repo) in self.repos.iter().enumerate() {
            match repo.family_listing(name) {
                FamilyListing::Absent => {}
                FamilyListing::NeedsPython(why) => return Err(why),
                FamilyListing::Versions(vs) => listings.push((idx, vs)),
            }
        }
        // First repository wins on equal versions; then the range filter.
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        let mut selected = Vec::new();
        for (idx, versions) in listings {
            for v in versions {
                let toks: Vec<String> = rez_version_tokens(&v)
                    .expect("validated in listing")
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                if !seen.insert(toks) {
                    continue;
                }
                if let Some(r) = range {
                    match RerVersion::try_from(v.as_str()) {
                        Ok(rv) if r.contains(&rv) => {}
                        Ok(_) => continue,
                        Err(_) => return Err(format!("version {v:?} not parseable by rer")),
                    }
                }
                selected.push((idx, v));
            }
        }
        Ok(Selection(selected))
    }

    /// The loading half of [`load_family`](Self::load_family): read and load
    /// every selected definition, in parallel. Order follows the selection.
    pub fn load_selection(&self, name: &str, selection: Selection) -> Vec<LoadedPackage> {
        selection
            .0
            .into_par_iter()
            .map(|(idx, v)| {
                let repo = &self.repos[idx];
                let dir = repo.location.join(name).join(&v);
                let (filepath, outcome) = repo.load_package(&dir);
                LoadedPackage {
                    name: name.to_string(),
                    version: v,
                    filepath,
                    outcome,
                }
            })
            .collect()
    }
}

/// Packages selected for loading: `(repository index, version dir)`.
#[derive(Clone, Debug, Default)]
pub struct Selection(pub Vec<(usize, String)>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_token_validation() {
        assert_eq!(rez_version_tokens("1.2.3"), Some(vec!["1", "2", "3"]));
        assert_eq!(rez_version_tokens("1-2+3"), Some(vec!["1", "2", "3"]));
        assert_eq!(rez_version_tokens("1_0"), Some(vec!["1_0"]));
        assert_eq!(rez_version_tokens("1..2"), None);
        assert_eq!(rez_version_tokens(".1"), None);
        assert_eq!(rez_version_tokens("1."), None);
        assert_eq!(rez_version_tokens(".building1.0"), None);
        assert_eq!(rez_version_tokens("..."), None);
    }

    #[test]
    fn package_names() {
        assert!(is_valid_package_name("foo"));
        assert!(is_valid_package_name("foo.bar_2"));
        assert!(!is_valid_package_name(".foo"));
        assert!(!is_valid_package_name("foo..bar"));
        assert!(!is_valid_package_name("__pycache__"));
        assert!(!is_valid_package_name("foo-bar"));
    }
}
