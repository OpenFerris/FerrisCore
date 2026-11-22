//! ISO 3166-1 country table, dial codes and trunk prefixes.
//!
//! This is data, not presentation: the labels the Desk renders are derived in
//! `meta.rs`, and there is no translation layer in this crate.

use serde::{Deserialize, Serialize};

macro_rules! define_countries {
    ($($variant:ident, $alpha2:expr, $alpha3:expr, $dial_code:expr);+ $(;)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum Country {
            $($variant),+
        }

        impl Country {
            pub const fn alpha2(&self) -> &'static str {
                match self {
                    $(Country::$variant => $alpha2),+
                }
            }

            pub const fn alpha3(&self) -> &'static str {
	            match self {
	                $(Country::$variant => $alpha3),+
	            }
            }

            /// Returns the dial code as a number (e.g., 1 for USA, 33 for France)
            pub const fn dial_code(&self) -> u16 {
                match self {
                    $(Country::$variant => $dial_code),+
                }
            }

            /// Returns the dial code formatted with + prefix (e.g., "+1", "+33")
            pub fn dial_code_formatted(&self) -> String {
                format!("+{}", self.dial_code())
            }

            pub fn from_alpha2(code: &str) -> Option<Self> {
                match code.to_uppercase().as_str() {
                    $($alpha2 => Some(Country::$variant)),+,
                    _ => None,
                }
            }

            pub fn from_alpha3(code: &str) -> Option<Self> {
            	match code.to_uppercase().as_str() {
                    $($alpha3 => Some(Country::$variant)),+,
                    _ => None,
                }
            }

            /// Returns a slice containing all countries.
            pub const fn all() -> &'static [Country] {
                &[$(Country::$variant),+]
            }
        }
    };
}

//Country list

define_countries![
    Afghanistan,                      "AF", "AFG",  93;
    Albania,                          "AL", "ALB",  355;
    Algeria,                          "DZ", "DZA",  213;
    Andorra,                          "AD", "AND",  376;
    Angola,                           "AO", "AGO",  244;
    AntiguaAndBarbuda,                "AG", "ATG",  1;
    Argentina,                        "AR", "ARG",  54;
    Armenia,                          "AM", "ARM",  374;
    Australia,                        "AU", "AUS",  61;
    Austria,                          "AT", "AUT",  43;
    Azerbaijan,                       "AZ", "AZE",  994;
    Bahamas,                          "BS", "BHS",  1;
    Bahrain,                          "BH", "BHR",  973;
    Bangladesh,                       "BD", "BGD",  880;
    Barbados,                         "BB", "BRB",  1;
    Belarus,                          "BY", "BLR",  375;
    Belgium,                          "BE", "BEL",  32;
    Belize,                           "BZ", "BLZ",  501;
    Benin,                            "BJ", "BEN",  229;
    Bhutan,                           "BT", "BTN",  975;
    Bolivia,                          "BO", "BOL",  591;
    BosniaAndHerzegovina,             "BA", "BIH",  387;
    Botswana,                         "BW", "BWA",  267;
    Brazil,                           "BR", "BRA",  55;
    Brunei,                           "BN", "BRN",  673;
    Bulgaria,                         "BG", "BGR",  359;
    BurkinaFaso,                      "BF", "BFA",  226;
    Burundi,                          "BI", "BDI",  257;
    Cambodia,                         "KH", "KHM",  855;
    Cameroon,                         "CM", "CMR",  237;
    Canada,                           "CA", "CAN",  1;
    CapeVerde,                        "CV", "CPV",  238;
    CentralAfricanRepublic,           "CF", "CAF",  236;
    Chad,                             "TD", "TCD",  235;
    Chile,                            "CL", "CHL",  56;
    China,                            "CN", "CHN",  86;
    Colombia,                         "CO", "COL",  57;
    Comoros,                          "KM", "COM",  269;
    CongoBrazzaville,                 "CG", "COG",  242;
    CongoKinshasa,                    "CD", "COD",  243;
    CostaRica,                        "CR", "CRI",  506;
    Croatia,                          "HR", "HRV",  385;
    Cuba,                             "CU", "CUB",  53;
    Cyprus,                           "CY", "CYP",  357;
    CzechRepublic,                    "CZ", "CZE",  420;
    Denmark,                          "DK", "DNK",  45;
    Djibouti,                         "DJ", "DJI",  253;
    Dominica,                         "DM", "DMA",  1;
    DominicanRepublic,                "DO", "DOM",  1;
    Ecuador,                          "EC", "ECU",  593;
    Egypt,                            "EG", "EGY",  20;
    ElSalvador,                       "SV", "SLV",  503;
    EquatorialGuinea,                 "GQ", "GNQ",  240;
    Eritrea,                          "ER", "ERI",  291;
    Estonia,                          "EE", "EST",  372;
    Eswatini,                         "SZ", "SWZ",  268;
    Ethiopia,                         "ET", "ETH",  251;
    Fiji,                             "FJ", "FJI",  679;
    Finland,                          "FI", "FIN",  358;
    France,                           "FR", "FRA",  33;
    Gabon,                            "GA", "GAB",  241;
    Gambia,                           "GM", "GMB",  220;
    Georgia,                          "GE", "GEO",  995;
    Germany,                          "DE", "DEU",  49;
    Ghana,                            "GH", "GHA",  233;
    Greece,                           "GR", "GRC",  30;
    Grenada,                          "GD", "GRD",  1;
    Guatemala,                        "GT", "GTM",  502;
    Guinea,                           "GN", "GIN",  224;
    GuineaBissau,                     "GW", "GNB",  245;
    Guyana,                           "GY", "GUY",  592;
    Haiti,                            "HT", "HTI",  509;
    Honduras,                         "HN", "HND",  504;
    Hungary,                          "HU", "HUN",  36;
    Iceland,                          "IS", "ISL",  354;
    India,                            "IN", "IND",  91;
    Indonesia,                        "ID", "IDN",  62;
    Iran,                             "IR", "IRN",  98;
    Iraq,                             "IQ", "IRQ",  964;
    Ireland,                          "IE", "IRL",  353;
    Israel,                           "IL", "ISR",  972;
    Italy,                            "IT", "ITA",  39;
    IvoryCoast,                       "CI", "CIV",  225;
    Jamaica,                          "JM", "JAM",  1;
    Japan,                            "JP", "JPN",  81;
    Jordan,                           "JO", "JOR",  962;
    Kazakhstan,                       "KZ", "KAZ",  7;
    Kenya,                            "KE", "KEN",  254;
    Kiribati,                         "KI", "KIR",  686;
    Kosovo,                           "XK", "XKX",  383;
    Kuwait,                           "KW", "KWT",  965;
    Kyrgyzstan,                       "KG", "KGZ",  996;
    Laos,                             "LA", "LAO",  856;
    Latvia,                           "LV", "LVA",  371;
    Lebanon,                          "LB", "LBN",  961;
    Lesotho,                          "LS", "LSO",  266;
    Liberia,                          "LR", "LBR",  231;
    Libya,                            "LY", "LBY",  218;
    Liechtenstein,                    "LI", "LIE",  423;
    Lithuania,                        "LT", "LTU",  370;
    Luxembourg,                       "LU", "LUX",  352;
    Madagascar,                       "MG", "MDG",  261;
    Malawi,                           "MW", "MWI",  265;
    Malaysia,                         "MY", "MYS",  60;
    Maldives,                         "MV", "MDV",  960;
    Mali,                             "ML", "MLI",  223;
    Malta,                            "MT", "MLT",  356;
    MarshallIslands,                  "MH", "MHL",  692;
    Mauritania,                       "MR", "MRT",  222;
    Mauritius,                        "MU", "MUS",  230;
    Mexico,                           "MX", "MEX",  52;
    Micronesia,                       "FM", "FSM",  691;
    Moldova,                          "MD", "MDA",  373;
    Monaco,                           "MC", "MCO",  377;
    Mongolia,                         "MN", "MNG",  976;
    Montenegro,                       "ME", "MNE",  382;
    Morocco,                          "MA", "MAR",  212;
    Mozambique,                       "MZ", "MOZ",  258;
    Myanmar,                          "MM", "MMR",  95;
    Namibia,                          "NA", "NAM",  264;
    Nauru,                            "NR", "NRU",  674;
    Nepal,                            "NP", "NPL",  977;
    Netherlands,                      "NL", "NLD",  31;
    NewZealand,                       "NZ", "NZL",  64;
    Nicaragua,                        "NI", "NIC",  505;
    Niger,                            "NE", "NER",  227;
    Nigeria,                          "NG", "NGA",  234;
    NorthKorea,                       "KP", "PRK",  850;
    NorthMacedonia,                   "MK", "MKD",  389;
    Norway,                           "NO", "NOR",  47;
    Oman,                             "OM", "OMN",  968;
    Pakistan,                         "PK", "PAK",  92;
    Palau,                            "PW", "PLW",  680;
    Palestine,                        "PS", "PSE",  970;
    Panama,                           "PA", "PAN",  507;
    PapuaNewGuinea,                   "PG", "PNG",  675;
    Paraguay,                         "PY", "PRY",  595;
    Peru,                             "PE", "PER",  51;
    Philippines,                      "PH", "PHL",  63;
    Poland,                           "PL", "POL",  48;
    Portugal,                         "PT", "PRT",  351;
    Qatar,                            "QA", "QAT",  974;
    Romania,                          "RO", "ROU",  40;
    Russia,                           "RU", "RUS",  7;
    Rwanda,                           "RW", "RWA",  250;
    SaintKittsAndNevis,               "KN", "KNA",  1;
    SaintLucia,                       "LC", "LCA",  1;
    SaintVincentAndTheGrenadines,     "VC", "VCT",  1;
    Samoa,                            "WS", "WSM",  685;
    SanMarino,                        "SM", "SMR",  378;
    SaoTomeAndPrincipe,               "ST", "STP",  239;
    SaudiArabia,                      "SA", "SAU",  966;
    Senegal,                          "SN", "SEN",  221;
    Serbia,                           "RS", "SRB",  381;
    Seychelles,                       "SC", "SYC",  248;
    SierraLeone,                      "SL", "SLE",  232;
    Singapore,                        "SG", "SGP",  65;
    Slovakia,                         "SK", "SVK",  421;
    Slovenia,                         "SI", "SVN",  386;
    SolomonIslands,                   "SB", "SLB",  677;
    Somalia,                          "SO", "SOM",  252;
    SouthAfrica,                      "ZA", "ZAF",  27;
    SouthKorea,                       "KR", "KOR",  82;
    SouthSudan,                       "SS", "SSD",  211;
    Spain,                            "ES", "ESP",  34;
    SriLanka,                         "LK", "LKA",  94;
    Sudan,                            "SD", "SDN",  249;
    Suriname,                         "SR", "SUR",  597;
    Sweden,                           "SE", "SWE",  46;
    Switzerland,                      "CH", "CHE",  41;
    Syria,                            "SY", "SYR",  963;
    Taiwan,                           "TW", "TWN",  886;
    Tajikistan,                       "TJ", "TJK",  992;
    Tanzania,                         "TZ", "TZA",  255;
    Thailand,                         "TH", "THA",  66;
    TimorLeste,                       "TL", "TLS",  670;
    Togo,                             "TG", "TGO",  228;
    Tonga,                            "TO", "TON",  676;
    TrinidadAndTobago,                "TT", "TTO",  1;
    Tunisia,                          "TN", "TUN",  216;
    Turkey,                           "TR", "TUR",  90;
    Turkmenistan,                     "TM", "TKM",  993;
    Tuvalu,                           "TV", "TUV",  688;
    Uganda,                           "UG", "UGA",  256;
    Ukraine,                          "UA", "UKR",  380;
    UnitedArabEmirates,               "AE", "ARE",  971;
    UnitedKingdom,                    "GB", "GBR",  44;
    UnitedStatesOfAmerica,            "US", "USA",  1;
    Uruguay,                          "UY", "URY",  598;
    Uzbekistan,                       "UZ", "UZB",  998;
    Vanuatu,                          "VU", "VUT",  678;
    VaticanCity,                      "VA", "VAT",  39;
    Venezuela,                        "VE", "VEN",  58;
    Vietnam,                          "VN", "VNM",  84;
    VirginIslandsBritish,             "VG", "VGB",  1;
    VirginIslandsUS,                  "VI", "VIR",  1;
    Yemen,                            "YE", "YEM",  967;
    Zambia,                           "ZM", "ZMB",  260;
    Zimbabwe,                         "ZW", "ZWE",  263;
];

impl Country {
    /// Returns the country name in English.
    ///
    /// Splits the PascalCase variant into words at a lower→upper transition
    /// only (so an acronym stays one word) and lowercases the English minor
    /// words the variants spell capitalised:
    /// `Country::UnitedStatesOfAmerica.name()` returns
    /// `"United States of America"`, not `"United States Of America"`.
    pub fn name(&self) -> String {
        const MINOR_WORDS: &[&str] = &["Of", "The", "And", "For", "In", "On", "At", "To"];
        let variant_name = format!("{self:?}");
        let mut words: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut previous_lowercase = false;
        for c in variant_name.chars() {
            if c.is_uppercase() && previous_lowercase {
                words.push(std::mem::take(&mut current));
            }
            current.push(c);
            previous_lowercase = c.is_lowercase() || c.is_ascii_digit();
        }
        words.push(current);
        words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index > 0 && MINOR_WORDS.contains(&word.as_str()) {
                    word.to_lowercase()
                } else {
                    word.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Converts a Country into a flag emoji.
    /// For example Country::Bahamas.flag_emoji() == "🇧🇸"
    pub fn flag_emoji(&self) -> String {
        self.alpha2()
            .chars()
            .flat_map(|c| {
                std::char::from_u32(0x1F1E6 + (c.to_ascii_uppercase() as u32 - 'A' as u32))
            })
            .collect()
    }

    /// Returns the trunk prefix stripped when dialing internationally.
    /// NANP (+1), Italy (+39), and a few others have no trunk prefix.
    pub const fn trunk_prefix(&self) -> Option<&'static str> {
        match self {
            // NANP — no trunk prefix
            Country::UnitedStatesOfAmerica
            | Country::Canada
            | Country::Bahamas
            | Country::Barbados
            | Country::DominicanRepublic
            | Country::Jamaica
            | Country::TrinidadAndTobago => None,

            // Italy/San Marino/Vatican — 0 is part of the subscriber number
            Country::Italy | Country::SanMarino | Country::VaticanCity => None,

            // Countries without trunk prefix
            Country::Denmark
            | Country::Norway
            | Country::Iceland
            | Country::Liechtenstein
            | Country::Monaco
            | Country::Andorra => None,

            // Most countries use "0" as trunk prefix
            _ => Some("0"),
        }
    }

    /// Resolve a dial code (without `+`) to a country.
    ///
    /// Shared dial codes (the NANP's `+1`) resolve to the first table entry;
    /// callers that need the exact country should keep it in the stored value.
    pub fn from_dial_code(code: &str) -> Option<Self> {
        Self::all()
            .iter()
            .copied()
            .find(|country| country.dial_code().to_string() == code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WS6: the doc promise — one space per word, not per capital letter.
    #[test]
    fn names_split_words_not_capitals() {
        assert_eq!(
            Country::UnitedStatesOfAmerica.name(),
            "United States of America"
        );
        assert_eq!(Country::UnitedKingdom.name(), "United Kingdom");
        assert_eq!(Country::Spain.name(), "Spain");
    }

    /// WS7: Kosovo has no official ISO 3166-1 alpha-3; the user-assigned
    /// element is `XKX` (the table used `XKS`).
    #[test]
    fn kosovo_uses_the_user_assigned_alpha3() {
        assert_eq!(Country::Kosovo.alpha3(), "XKX");
        assert_eq!(Country::from_alpha3("XKX"), Some(Country::Kosovo));
        assert_eq!(Country::from_alpha2("XK"), Some(Country::Kosovo));
    }

    #[test]
    fn dial_codes_resolve_back_to_a_country() {
        assert_eq!(Country::from_dial_code("33"), Some(Country::France));
        assert_eq!(Country::from_dial_code("34"), Some(Country::Spain));
        assert_eq!(Country::from_dial_code("999"), None);
    }

    #[test]
    fn alpha2_and_alpha3_round_trip() {
        for country in Country::all() {
            assert_eq!(
                Country::from_alpha2(country.alpha2()),
                Some(*country),
                "alpha2 for {country:?}"
            );
            assert_eq!(
                Country::from_alpha3(country.alpha3()),
                Some(*country),
                "alpha3 for {country:?}"
            );
        }
    }
}
