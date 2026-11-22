use super::pattern::date_token;
use super::*;
use chrono::Utc;
use ferris_core::Value;
use ferris_db::{Db, Stmt};
use std::collections::HashMap;

#[test]
fn test_format_series_with_counter() {
    assert_eq!(format_series("TEST-.#####", 1), "TEST-00001");
    assert_eq!(format_series("TEST-.#####", 42), "TEST-00042");
}

#[test]
fn test_format_series_with_date() {
    let name = format_series("SO-.YY.-.#####", 7);
    assert!(name.starts_with("SO-"), "{name}");
    assert!(name.ends_with("00007"), "{name}");
}

/// A dot-separated Expression concatenates its parts without an extra
/// separator (Frappe's `parse_naming_series` appends each part directly);
/// the literal hyphens in the pattern are what produce the final dashes.
#[test]
fn expression_concatenates_parts_and_pads_the_counter() {
    let mut fields = HashMap::new();
    fields.insert("fieldname1".to_string(), Value::from("ACME"));
    assert_eq!(
        format_expression("EXAMPLE-.test-.{fieldname1}.-.#####", &fields, 7),
        "EXAMPLE-test-ACME-00007"
    );
}

/// The old-style form keeps the literal text and substitutes braces in
/// place, with the `#` run deciding the counter width.
#[test]
fn old_style_expression_substitutes_in_place() {
    let mut fields = HashMap::new();
    fields.insert("customer".to_string(), Value::from("ACME"));
    assert_eq!(
        format_expression_old_style("EXAMPLE-{customer}-{#####}", &fields, 42),
        "EXAMPLE-ACME-00042"
    );
    assert_eq!(
        format_expression_old_style("P-{customer}-{###}", &fields, 3),
        "P-ACME-003"
    );
}

/// An unknown brace token stays visible instead of vanishing, so a typo in
/// a company's pattern is not silently erased from every name.
#[test]
fn an_unknown_brace_token_stays_literal() {
    let fields = HashMap::new();
    assert_eq!(
        format_expression_old_style("X-{nope}-{#####}", &fields, 1),
        "X-{nope}-00001"
    );
}

/// Frappe's `Random` name is ten lowercase base32hex characters.
#[test]
fn random_names_are_ten_base32hex_characters() {
    for _ in 0..32 {
        let name = random_name();
        assert_eq!(name.len(), 10, "{name}");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c)),
            "unexpected character in {name}"
        );
    }
}

/// An amendment takes the next free `NAME-<n>`, never reusing a number and
/// ignoring rows that merely share the prefix.
#[tokio::test]
async fn amend_names_take_the_next_counter() {
    let db = ferris_db::memory_db().await.unwrap();
    db.ensure_table("tabTicket", &ferris_schema::migration::document_columns())
        .await
        .unwrap();
    for name in ["T-1", "T-1-1", "T-1-2", "T-1-4", "T-10-other"] {
        db.execute(&Stmt::new(
            "INSERT INTO tabTicket (name, owner, docstatus, creation, modified, data) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                Value::from(name),
                Value::from("admin@local"),
                Value::from(0_i64),
                Value::from(0_i64),
                Value::from(0_i64),
                Value::from("{}"),
            ],
        ))
        .await
        .unwrap();
    }
    assert_eq!(
        next_amend_name(&db, "Ticket", "T-1").await.unwrap(),
        "T-1-5"
    );
    assert_eq!(
        next_amend_name(&db, "Ticket", "T-2").await.unwrap(),
        "T-2-1"
    );
}

/// `MM` is the month and `mm` the minute; the case-sensitive token match is
/// the only thing that keeps them apart.
#[test]
fn date_tokens_are_case_sensitive() {
    let now = Utc::now();
    assert_eq!(date_token("MM", now), Some(now.format("%m").to_string()));
    assert_eq!(date_token("mm", now), Some(now.format("%M").to_string()));
    assert_eq!(
        date_token("timestamp", now),
        Some(now.timestamp().to_string())
    );
    assert_eq!(date_token("nope", now), None);
}

async fn db() -> Db {
    ferris_db::memory_db().await.unwrap()
}

/// The counter has to keep climbing across calls and across processes, so it
/// lives in the database rather than in a process-local counter.
#[tokio::test]
async fn next_is_monotonic_and_shared() {
    let db = db().await;
    assert_eq!(
        NamingSeriesStore::current(&db, "S-.#####").await.unwrap(),
        0
    );

    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(NamingSeriesStore::next(&db, "S-.#####").await.unwrap());
    }
    assert_eq!(seen, vec!["S-00001", "S-00002", "S-00003"]);
    assert_eq!(
        NamingSeriesStore::current(&db, "S-.#####").await.unwrap(),
        3
    );

    // A second series has its own counter, so they cannot collide.
    assert_eq!(
        NamingSeriesStore::next(&db, "T-.#####").await.unwrap(),
        "T-00001"
    );
}

/// The read-modify-write is in a transaction, so concurrent callers get
/// distinct names rather than the same one twice.
#[tokio::test]
async fn concurrent_callers_get_distinct_names() {
    let db = db().await;
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let db = db.clone();
            tokio::spawn(async move { NamingSeriesStore::next(&db, "C-.#####").await })
        })
        .collect();

    let mut names = Vec::with_capacity(tasks.len());
    for task in tasks {
        names.push(task.await.unwrap().unwrap());
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 8, "names collided: {names:?}");
}
