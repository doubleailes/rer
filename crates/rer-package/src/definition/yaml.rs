//! `rez.serialise.load_yaml` — not implemented yet: every file is handed
//! back to rez.

use super::value::{needs, LoadOutcome, NeedsPythonReason};

/// Load a `package.yaml` from its raw bytes, like `rez.serialise.load_yaml`.
pub fn load_package_yaml(_bytes: &[u8]) -> LoadOutcome {
    LoadOutcome::NeedsPython(needs(
        NeedsPythonReason::Yaml,
        None,
        "YAML loading not implemented yet",
    ))
}
