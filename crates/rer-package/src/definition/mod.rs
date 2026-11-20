//! Full package-definition loading: the dict rez's `load_from_file`
//! produces for a `package.py` / `package.yaml`, or an explicit
//! [`NeedsPython`] when only rez's Python loader can produce it.
//!
//! This is an addition next to the four-field fast parser
//! ([`crate::parse_static_package_py`]), which is unchanged.

mod pyload;
mod source;
mod tokenize;
mod value;
mod yaml;

pub use pyload::load_package_py;
pub use value::{
    Decorator, LoadOutcome, NeedsPython, NeedsPythonReason, PackageDefinition, SourceCode, Value,
};
pub use yaml::load_package_yaml;
