pub mod query;

pub use query::{QUERY, QueryUtils};

// The locale tables and date helpers are pure data, so they live one crate
// further down; re-exported here because the components reach for them through
// `crate::components::utils`.
pub use ferris_web_shared::{country, date, phone_number};
