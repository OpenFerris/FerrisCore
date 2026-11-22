//! Front matter parsing, shared by the renderer and the website routes.

pub mod front_matter;

pub use front_matter::{FrontMatter, parse_front_matter};
