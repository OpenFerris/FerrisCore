use crate::country::Country;

use super::format::PhoneFormat;

/// A phone number, stored as its **national subscriber** digits.
///
/// The dial code is not part of the value: it is a property of the country the
/// number belongs to, and [`PhoneNumber::format_international`] puts it back on
/// display. Keeping the dial code in the digits made the stored value grow by
/// one country code on every load of the form (WS5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhoneNumber(String);

impl PhoneNumber {
    /// Creates a new PhoneNumber from raw digits, keeping at most `max_digits`.
    ///
    /// This is the country-less constructor: it does not strip a dial code or a
    /// trunk prefix. Use [`PhoneNumber::new_in_country`] for a value that may
    /// carry either.
    pub fn new(input: &str, max_digits: usize) -> Self {
        Self(
            input
                .chars()
                .filter(|c| c.is_ascii_digit())
                .take(max_digits)
                .collect(),
        )
    }

    /// Creates a PhoneNumber from a value that may be written in international
    /// (`+34 612 345 678`, `0034…`, `34612345678`) or national (`0612…`) form,
    /// keeping only the subscriber digits.
    ///
    /// This is what makes the stored value round-trip: `format_international`
    /// emits the dial code, and this constructor takes it back off instead of
    /// treating it as subscriber digits.
    pub fn new_in_country(input: &str, country: Country) -> Self {
        let (phone, _) = Self::parse(input, country);
        phone
    }

    /// Parse a stored value into its subscriber digits and the country it
    /// belongs to.
    ///
    /// A `+<dial>` or `00<dial>` prefix identifies the country; without one the
    /// value is read in `default_country`. The national trunk prefix (`0` in
    /// most countries) is dropped, and digits left beyond the country's
    /// national length are clamped — which also repairs values the old
    /// re-prefixing bug already corrupted (`+34 34 612…`).
    pub fn parse(input: &str, default_country: Country) -> (Self, Country) {
        let digits: String = input.chars().filter(char::is_ascii_digit).collect();
        let trimmed = input.trim_start();
        let international = trimmed.starts_with('+') || digits.starts_with("00");
        let without_intl_prefix = digits.strip_prefix("00").unwrap_or(&digits);

        let country = if international {
            // Longest dial-code prefix wins; a bare number can begin with a
            // digit sequence that merely looks like a dial code, so the match
            // must also leave something behind as the national number.
            Country::all()
                .iter()
                .copied()
                .filter(|candidate| {
                    let dial = candidate.dial_code().to_string();
                    without_intl_prefix.starts_with(&dial) && without_intl_prefix.len() > dial.len()
                })
                .max_by_key(|candidate| candidate.dial_code().to_string().len())
                .unwrap_or(default_country)
        } else {
            default_country
        };

        let dial = country.dial_code().to_string();
        let mut subscriber = without_intl_prefix;
        // Drop the international dial code, at most twice: the second pass
        // repairs a value the old code already re-prefixed.
        for _ in 0..2 {
            if let Some(stripped) = subscriber.strip_prefix(&dial) {
                subscriber = stripped;
            }
        }
        if let Some(trunk) = country.trunk_prefix() {
            subscriber = subscriber.strip_prefix(trunk).unwrap_or(subscriber);
        }

        let max_digits = PhoneFormat::for_country(country).max_digits;
        let subscriber: String = subscriber.chars().take(max_digits).collect();
        (Self(subscriber), country)
    }

    /// Returns the raw digits as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns true if empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Formats the phone number according to the given country's format.
    pub fn format(&self, country: Country) -> String {
        PhoneFormat::for_country(country).format(&self.0)
    }

    /// Returns the full international format: "+33 6 12 34 56 78"
    pub fn format_international(&self, country: Country) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        let subscriber = match country.trunk_prefix() {
            Some(prefix) => self.0.strip_prefix(prefix).unwrap_or(&self.0),
            None => &self.0,
        };
        format!(
            "{} {}",
            country.dial_code_formatted(),
            PhoneFormat::for_country(country).format(subscriber)
        )
    }
}
