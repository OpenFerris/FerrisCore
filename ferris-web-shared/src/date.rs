//! Date helpers shared by the date pickers.
//!
//! The pickers read a `YYYY-MM-DD` value from the URL and step month by month;
//! both are pure date arithmetic, so they live in the browser-neutral crate.

use time::{Date, Month};

pub struct DateUtils;

impl DateUtils {
    /// Parse a `YYYY-MM-DD` value from the URL.
    ///
    /// Takes the query value by reference: the caller owns the `Option<String>`
    /// and the parser only reads it, so no clone is needed per call (WS10).
    pub fn parse_from_url(date_str: Option<&str>) -> Option<Date> {
        let s = date_str?;
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 3 {
            return None;
        }
        let year = parts.first()?.parse::<i32>().ok()?;
        let month = Month::try_from(parts.get(1)?.parse::<u8>().ok()?).ok()?;
        let day = parts.get(2)?.parse::<u8>().ok()?;
        Date::from_calendar_date(year, month, day).ok()
    }

    /// Get the next month and year, handling December -> January rollover
    pub fn next_month_year(month: Month, year: i32) -> (Month, i32) {
        if month == Month::December {
            (Month::January, year + 1)
        } else {
            (month.next(), year)
        }
    }

    /// Get the previous month and year, handling January -> December rollover
    pub fn prev_month_year(month: Month, year: i32) -> (Month, i32) {
        if month == Month::January {
            (Month::December, year - 1)
        } else {
            (month.previous(), year)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_url_date() {
        let parsed = DateUtils::parse_from_url(Some("2026-02-28")).expect("valid date");
        assert_eq!(parsed.year(), 2026);
        assert_eq!(parsed.month(), Month::February);
        assert_eq!(parsed.day(), 28);
    }

    #[test]
    fn rejects_missing_or_malformed_values() {
        assert!(DateUtils::parse_from_url(None).is_none());
        assert!(DateUtils::parse_from_url(Some("")).is_none());
        assert!(DateUtils::parse_from_url(Some("2026-02")).is_none());
        assert!(DateUtils::parse_from_url(Some("2026-13-01")).is_none());
        // 2026 is not a leap year.
        assert!(DateUtils::parse_from_url(Some("2026-02-29")).is_none());
        assert!(DateUtils::parse_from_url(Some("not-a-date")).is_none());
    }

    #[test]
    fn month_steps_roll_over_the_year() {
        assert_eq!(
            DateUtils::next_month_year(Month::December, 2026),
            (Month::January, 2027)
        );
        assert_eq!(
            DateUtils::prev_month_year(Month::January, 2026),
            (Month::December, 2025)
        );
        assert_eq!(
            DateUtils::next_month_year(Month::May, 2026),
            (Month::June, 2026)
        );
        assert_eq!(
            DateUtils::prev_month_year(Month::May, 2026),
            (Month::April, 2026)
        );
    }
}
