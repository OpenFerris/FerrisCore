//! Unit tests for row decoding.
//!
//! Restored from the inline `#[cfg(test)] mod tests` moved into `tests/`
//! by commit 2013f3d.

use super::*;
use crate::test_support::row_of;

#[tokio::test]
async fn missing_column_reads_as_none_not_an_error() {
    let row = Row::new(row_of(&[("name", "'T-1'")]).await.unwrap());
    assert!(row.has("name"));
    assert!(row.has("NAME"));
    assert!(row.opt_str("nope").is_none());
    assert!(row.opt_i64("nope").is_none());
    assert!(row.opt_bool("nope").is_none());
    assert!(row.str("nope").is_err());
    assert_eq!(row.str("name").unwrap(), "T-1");
    assert_eq!(row.len(), 1);
}

// 7.0 is exactly representable, so an equality check is meaningful here.
#[allow(clippy::float_cmp)]
#[tokio::test]
async fn integers_decode_from_several_widths() {
    let row = Row::new(
        row_of(&[("a", "7"), ("b", "CAST(7 AS INTEGER)")])
            .await
            .unwrap(),
    );
    assert_eq!(row.i64("a").unwrap(), 7);
    assert_eq!(row.i64("b").unwrap(), 7);
    assert_eq!(row.opt_f64("a").unwrap(), 7.0);
}

#[tokio::test]
async fn counts_are_clamped_at_zero() {
    let row = Row::new(row_of(&[("count", "0")]).await.unwrap());
    assert_eq!(row.count("count"), 0);
    let row = Row::new(row_of(&[("count", "12")]).await.unwrap());
    assert_eq!(row.count("count"), 12);
    // An absent column counts as zero rather than failing.
    let row = Row::new(row_of(&[("other", "1")]).await.unwrap());
    assert_eq!(row.count("count"), 0);
}

#[tokio::test]
async fn json_decodes_from_text() {
    let row = Row::new(row_of(&[("data", "'{\"name\":\"T-1\"}'")]).await.unwrap());
    let value = row.json("data").unwrap();
    assert_eq!(value.get("name").and_then(|v| v.as_str()), Some("T-1"));
    assert!(row.json("absent").is_err());
    assert!(row.opt_json("absent").unwrap().is_none());
}

#[tokio::test]
async fn json_rejects_non_json_text() {
    let row = Row::new(row_of(&[("data", "'not json'")]).await.unwrap());
    assert!(row.json("data").is_err());
}

#[tokio::test]
async fn booleans_accept_the_integer_shape_sqlite_uses() {
    // SQLite applies INTEGER affinity to a BOOLEAN declared column, which
    // is why the bare `bool` read in `sharing.rs` failed there.
    let row = Row::new(row_of(&[("flag", "1"), ("off", "0")]).await.unwrap());
    assert!(row.bool("flag").unwrap());
    assert!(!row.bool("off").unwrap());
    assert!(row.bool("absent").is_err());
}

#[tokio::test]
async fn booleans_accept_the_text_shape() {
    let row = Row::new(
        row_of(&[("a", "'true'"), ("b", "'0'"), ("c", "'FALSE'")])
            .await
            .unwrap(),
    );
    assert!(row.bool("a").unwrap());
    assert!(!row.bool("b").unwrap());
    assert!(!row.bool("c").unwrap());
}
