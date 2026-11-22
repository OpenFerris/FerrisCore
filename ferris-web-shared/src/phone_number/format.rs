use crate::country::Country;

/// Phone number formatting rules for a country.
pub struct PhoneFormat {
    /// Digit group sizes (e.g., [2, 2, 2, 2, 2] for France "06 12 34 56 78")
    pub groups: &'static [usize],
    /// Maximum digits allowed (excluding country code)
    pub max_digits: usize,
}

impl PhoneFormat {
    /// Returns the phone format for a given country.
    pub const fn for_country(country: Country) -> Self {
        match country {
            // North America (NANP): 3-3-4, 10 digits
            Country::UnitedStatesOfAmerica
            | Country::Canada
            | Country::Bahamas
            | Country::Barbados
            | Country::DominicanRepublic
            | Country::Jamaica
            | Country::TrinidadAndTobago => Self {
                groups: &[3, 3, 4],
                max_digits: 10,
            },

            // France: 9 subscriber digits (6/7 XX XX XX XX) — trunk 0 dropped with +33
            Country::France => Self {
                groups: &[1, 2, 2, 2, 2],
                max_digits: 9,
            },

            // UK: 10 subscriber digits (7XXX XXX XXX) — trunk 0 dropped with +44
            Country::UnitedKingdom => Self {
                groups: &[4, 3, 3],
                max_digits: 10,
            },

            // Germany: 10 subscriber digits (15X XXX XXXX) — trunk 0 dropped with +49
            Country::Germany => Self {
                groups: &[3, 3, 4],
                max_digits: 10,
            },

            // Thailand: 9 subscriber digits (8X XXX XXXX) — trunk 0 dropped with +66
            Country::Thailand => Self {
                groups: &[2, 3, 4],
                max_digits: 9,
            },

            // Japan: 10 subscriber digits (90 XXXX XXXX) — trunk 0 dropped with +81
            Country::Japan => Self {
                groups: &[2, 4, 4],
                max_digits: 10,
            },

            // China: 3-4-4, 11 digits
            Country::China => Self {
                groups: &[3, 4, 4],
                max_digits: 11,
            },

            // India: 5-5, 10 digits
            Country::India => Self {
                groups: &[5, 5],
                max_digits: 10,
            },

            // Australia: 3-3-3, 9 digits
            Country::Australia => Self {
                groups: &[3, 3, 3],
                max_digits: 9,
            },

            // Brazil: 2-5-4, 11 digits
            Country::Brazil => Self {
                groups: &[2, 5, 4],
                max_digits: 11,
            },

            // Mexico: 2-4-4, 10 digits
            Country::Mexico => Self {
                groups: &[2, 4, 4],
                max_digits: 10,
            },

            // Spain: 3-3-3, 9 digits
            Country::Spain => Self {
                groups: &[3, 3, 3],
                max_digits: 9,
            },

            // Italy: 3-3-4, 10 digits
            Country::Italy => Self {
                groups: &[3, 3, 4],
                max_digits: 10,
            },

            // Default: 3-3-4, 15 digits (ITU-T E.164 max)
            _ => Self {
                groups: &[3, 3, 4],
                max_digits: 15,
            },
        }
    }

    /// Formats a raw digit string according to the format groups.
    pub fn format(&self, digits: &str) -> String {
        let mut result = String::new();
        let mut chars = digits.chars().peekable();

        for (i, &group_size) in self.groups.iter().enumerate() {
            if chars.peek().is_none() {
                break;
            }

            if i > 0 {
                result.push(' ');
            }

            for _ in 0..group_size {
                if let Some(c) = chars.next() {
                    result.push(c);
                } else {
                    break;
                }
            }
        }

        // Append any remaining digits (for countries with variable length)
        for c in chars {
            result.push(c);
        }

        result
    }

    /// Returns a placeholder string showing the format pattern.
    /// Example: "06 12 34 56 78" for France.
    pub fn placeholder(&self) -> String {
        let digits: String = (0..self.max_digits)
            .map(|i| char::from(b'0' + (i % 10) as u8))
            .collect();
        self.format(&digits)
    }
}
