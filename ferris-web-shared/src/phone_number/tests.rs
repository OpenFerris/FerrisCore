use super::*;
use crate::country::Country;

// ─────────────────────────────────────────────────────────────
// Format tests
// ─────────────────────────────────────────────────────────────

#[test]
fn test_france_format() {
    let format = PhoneFormat::for_country(Country::France);
    // Subscriber digits only — user sees +33 in the dropdown
    assert_eq!(format.format("612345678"), "6 12 34 56 78");
    assert_eq!(format.format("712345678"), "7 12 34 56 78");
    assert_eq!(format.format("61234"), "6 12 34");
    assert_eq!(format.max_digits, 9);
}

#[test]
fn test_usa_format() {
    let format = PhoneFormat::for_country(Country::UnitedStatesOfAmerica);
    assert_eq!(format.format("5551234567"), "555 123 4567");
    assert_eq!(format.format("555123"), "555 123");
    assert_eq!(format.format("555"), "555");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_canada_format() {
    let format = PhoneFormat::for_country(Country::Canada);
    assert_eq!(format.format("4161234567"), "416 123 4567");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_uk_format() {
    let format = PhoneFormat::for_country(Country::UnitedKingdom);
    assert_eq!(format.format("7123456789"), "7123 456 789");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_germany_format() {
    let format = PhoneFormat::for_country(Country::Germany);
    assert_eq!(format.format("1512345678"), "151 234 5678");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_thailand_format() {
    let format = PhoneFormat::for_country(Country::Thailand);
    assert_eq!(format.format("812345678"), "81 234 5678");
    assert_eq!(format.max_digits, 9);
}

#[test]
fn test_japan_format() {
    let format = PhoneFormat::for_country(Country::Japan);
    assert_eq!(format.format("9012345678"), "90 1234 5678");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_china_format() {
    let format = PhoneFormat::for_country(Country::China);
    assert_eq!(format.format("13812345678"), "138 1234 5678");
    assert_eq!(format.max_digits, 11);
}

#[test]
fn test_india_format() {
    let format = PhoneFormat::for_country(Country::India);
    assert_eq!(format.format("9876543210"), "98765 43210");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_australia_format() {
    let format = PhoneFormat::for_country(Country::Australia);
    assert_eq!(format.format("412345678"), "412 345 678");
    assert_eq!(format.max_digits, 9);
}

#[test]
fn test_brazil_format() {
    let format = PhoneFormat::for_country(Country::Brazil);
    assert_eq!(format.format("11987654321"), "11 98765 4321");
    assert_eq!(format.max_digits, 11);
}

#[test]
fn test_mexico_format() {
    let format = PhoneFormat::for_country(Country::Mexico);
    assert_eq!(format.format("5512345678"), "55 1234 5678");
    assert_eq!(format.max_digits, 10);
}

#[test]
fn test_spain_format() {
    let format = PhoneFormat::for_country(Country::Spain);
    assert_eq!(format.format("612345678"), "612 345 678");
    assert_eq!(format.max_digits, 9);
}

#[test]
fn test_italy_format() {
    let format = PhoneFormat::for_country(Country::Italy);
    assert_eq!(format.format("3123456789"), "312 345 6789");
    assert_eq!(format.max_digits, 10);
}

// ─────────────────────────────────────────────────────────────
// Edge cases
// ─────────────────────────────────────────────────────────────

#[test]
fn test_empty_input() {
    let format = PhoneFormat::for_country(Country::France);
    assert_eq!(format.format(""), "");
}

#[test]
fn test_single_digit() {
    let format = PhoneFormat::for_country(Country::France);
    assert_eq!(format.format("0"), "0");
}

#[test]
fn test_partial_group() {
    let format = PhoneFormat::for_country(Country::France);
    assert_eq!(format.format("61"), "6 1");
    assert_eq!(format.format("612"), "6 12");
    assert_eq!(format.format("6123"), "6 12 3");
}

#[test]
fn test_default_format_for_unknown_country() {
    // Countries without specific format should use default 3-3-4
    let format = PhoneFormat::for_country(Country::Afghanistan);
    assert_eq!(format.format("1234567890"), "123 456 7890");
    assert_eq!(format.max_digits, 15);
}

#[test]
fn test_overflow_digits_appended() {
    // Extra digits beyond format should be appended
    let format = PhoneFormat::for_country(Country::Spain);
    // Spain is 3-3-3 (9 digits), but if we pass more they get appended
    assert_eq!(format.format("123456789012"), "123 456 789012");
}

// ─────────────────────────────────────────────────────────────
// NANP countries share same format
// ─────────────────────────────────────────────────────────────

#[test]
fn test_nanp_countries_same_format() {
    let countries = [
        Country::UnitedStatesOfAmerica,
        Country::Canada,
        Country::Bahamas,
        Country::Barbados,
        Country::DominicanRepublic,
        Country::Jamaica,
        Country::TrinidadAndTobago,
    ];

    for country in countries {
        let format = PhoneFormat::for_country(country);
        assert_eq!(format.groups, &[3, 3, 4], "NANP format for {:?}", country);
        assert_eq!(format.max_digits, 10, "NANP max_digits for {:?}", country);
    }
}

// ─────────────────────────────────────────────────────────────
// PhoneNumber tests
// ─────────────────────────────────────────────────────────────

#[test]
fn test_phone_number_filters_non_digits() {
    let phone = PhoneNumber::new("06 12-34.56", 10);
    assert_eq!(phone.as_str(), "06123456");
}

#[test]
fn test_phone_number_respects_max_digits() {
    let phone = PhoneNumber::new("0612345678901234", 10);
    assert_eq!(phone.as_str(), "0612345678");
}

#[test]
fn test_phone_number_format() {
    let phone = PhoneNumber::new("612345678", 9);
    assert_eq!(phone.format(Country::France), "6 12 34 56 78");
}

#[test]
fn test_phone_number_default_is_empty() {
    let phone = PhoneNumber::default();
    assert_eq!(phone.as_str(), "");
    assert!(phone.is_empty());
}

#[test]
fn test_phone_number_format_international() {
    let phone = PhoneNumber::new("612345678", 9);
    assert_eq!(
        phone.format_international(Country::France),
        "+33 6 12 34 56 78"
    );
}

#[test]
fn test_phone_number_format_international_empty() {
    let phone = PhoneNumber::default();
    assert_eq!(phone.format_international(Country::France), "");
}

#[test]
fn test_placeholder_france() {
    let format = PhoneFormat::for_country(Country::France);
    assert_eq!(format.placeholder(), "0 12 34 56 78");
}

#[test]
fn test_format_international_no_double_trunk() {
    let cases = [
        (Country::UnitedKingdom, "7123456789", "+44 7123 456 789"),
        (Country::Germany, "1512345678", "+49 151 234 5678"),
        (Country::France, "612345678", "+33 6 12 34 56 78"),
        (Country::France, "712345678", "+33 7 12 34 56 78"),
        (Country::Japan, "9012345678", "+81 90 1234 5678"),
        (Country::Thailand, "812345678", "+66 81 234 5678"),
    ];
    for (country, digits, expected) in cases {
        let phone = PhoneNumber::new(digits, PhoneFormat::for_country(country).max_digits);
        assert_eq!(
            phone.format_international(country),
            expected,
            "Failed for {:?}",
            country
        );
    }
}

#[test]
fn test_format_international_no_trunk_countries_unchanged() {
    let phone_us = PhoneNumber::new("5551234567", 10);
    assert_eq!(
        phone_us.format_international(Country::UnitedStatesOfAmerica),
        "+1 555 123 4567"
    );
    let phone_es = PhoneNumber::new("612345678", 9);
    assert_eq!(
        phone_es.format_international(Country::Spain),
        "+34 612 345 678"
    );
    let phone_it = PhoneNumber::new("3123456789", 10);
    assert_eq!(
        phone_it.format_international(Country::Italy),
        "+39 312 345 6789"
    );
}

#[test]
fn test_placeholder_usa() {
    let format = PhoneFormat::for_country(Country::UnitedStatesOfAmerica);
    assert_eq!(format.placeholder(), "012 345 6789");
}

// ─────────────────────────────────────────────────────────────
// Round trip
// ─────────────────────────────────────────────────────────────

/// WS5: formatting and parsing must be inverses. The old code kept the
/// dial code as subscriber digits, so every load of the form prepended
/// another country code.
#[test]
fn international_format_round_trips() {
    let cases = [
        (Country::Spain, "+34 612 345 678"),
        (Country::France, "+33 6 12 34 56 78"),
        (Country::UnitedKingdom, "+44 7123 456 789"),
        (Country::UnitedStatesOfAmerica, "+1 555 123 4567"),
        (Country::Japan, "+81 90 1234 5678"),
    ];
    for (country, formatted) in cases {
        let phone = PhoneNumber::new_in_country(formatted, country);
        assert_eq!(
            phone.format_international(country),
            formatted,
            "round trip for {country:?}"
        );
        // And it stays stable when loaded again.
        let again = PhoneNumber::new_in_country(&phone.format_international(country), country);
        assert_eq!(again, phone, "second round trip for {country:?}");
    }
}

#[test]
fn parsing_recovers_the_country_from_the_dial_code() {
    let (phone, country) = PhoneNumber::parse("+44 7123 456 789", Country::Spain);
    assert_eq!(country, Country::UnitedKingdom);
    assert_eq!(phone.as_str(), "7123456789");
    assert_eq!(phone.format_international(country), "+44 7123 456 789");
}

#[test]
fn national_form_drops_the_trunk_prefix() {
    let phone = PhoneNumber::new_in_country("0612 345 678", Country::Spain);
    assert_eq!(phone.as_str(), "612345678");
    let phone = PhoneNumber::new_in_country("0033 6 12 34 56 78", Country::Spain);
    assert_eq!(phone.as_str(), "612345678");
}

/// A value the old code already corrupted (`+34 34 612 345 678`) is
/// repaired on load instead of growing again.
#[test]
fn an_already_doubled_dial_code_is_repaired() {
    let phone = PhoneNumber::new_in_country("+34 34 612 345 678", Country::Spain);
    assert_eq!(phone.as_str(), "612345678");
    assert_eq!(
        phone.format_international(Country::Spain),
        "+34 612 345 678"
    );
}

#[test]
fn an_empty_value_stays_empty() {
    let (phone, country) = PhoneNumber::parse("", Country::France);
    assert!(phone.is_empty());
    assert_eq!(country, Country::France);
    assert_eq!(phone.format_international(Country::France), "");
}
