//! Cities. The big metros are known by every name people use for them, so
//! "blr", "Bangalore" and "Bengaluru" land in one pool. Any other city is
//! accepted as typed: listings are open across India.

use crate::geo::GeoPoint;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A city in its canonical spelling.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct City(String);

impl TryFrom<String> for City {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        City::parse(&value).ok_or_else(|| format!("not a city name: {value:?}"))
    }
}

impl From<City> for String {
    fn from(city: City) -> Self {
        city.0
    }
}

struct Known {
    name: &'static str,
    aliases: &'static [&'static str],
    centre: (f64, f64),
}

/// The metros the map opens on. A satellite city is its own pool: people
/// search Gurugram and Noida apart from Delhi.
const KNOWN: &[Known] = &[
    Known {
        name: "Bengaluru",
        aliases: &["bengaluru", "bangalore", "blr", "bengalooru", "banglore"],
        centre: (12.9716, 77.5946),
    },
    Known { name: "Mumbai", aliases: &["mumbai", "bombay", "bom"], centre: (19.0760, 72.8777) },
    Known { name: "Navi Mumbai", aliases: &["navi mumbai"], centre: (19.0330, 73.0297) },
    Known { name: "Thane", aliases: &["thane"], centre: (19.2183, 72.9781) },
    Known { name: "Delhi", aliases: &["delhi", "new delhi", "delhi ncr", "ncr"], centre: (28.6139, 77.2090) },
    Known { name: "Gurugram", aliases: &["gurugram", "gurgaon", "ggn"], centre: (28.4595, 77.0266) },
    Known { name: "Noida", aliases: &["noida", "greater noida"], centre: (28.5355, 77.3910) },
    Known { name: "Ghaziabad", aliases: &["ghaziabad"], centre: (28.6692, 77.4538) },
    Known { name: "Pune", aliases: &["pune", "poona"], centre: (18.5204, 73.8567) },
    Known { name: "Hyderabad", aliases: &["hyderabad", "hyd", "secunderabad"], centre: (17.3850, 78.4867) },
    Known { name: "Chennai", aliases: &["chennai", "madras"], centre: (13.0827, 80.2707) },
    Known { name: "Kolkata", aliases: &["kolkata", "calcutta"], centre: (22.5726, 88.3639) },
    Known { name: "Ahmedabad", aliases: &["ahmedabad", "amdavad"], centre: (23.0225, 72.5714) },
    Known { name: "Jaipur", aliases: &["jaipur"], centre: (26.9124, 75.7873) },
    Known { name: "Chandigarh", aliases: &["chandigarh", "mohali", "panchkula"], centre: (30.7333, 76.7794) },
    Known { name: "Lucknow", aliases: &["lucknow"], centre: (26.8467, 80.9462) },
    Known { name: "Indore", aliases: &["indore"], centre: (22.7196, 75.8577) },
    Known { name: "Kochi", aliases: &["kochi", "cochin", "ernakulam"], centre: (9.9312, 76.2673) },
    Known { name: "Coimbatore", aliases: &["coimbatore"], centre: (11.0168, 76.9558) },
    Known { name: "Bhubaneswar", aliases: &["bhubaneswar", "bhubaneshwar"], centre: (20.2961, 85.8245) },
    Known { name: "Nagpur", aliases: &["nagpur"], centre: (21.1458, 79.0882) },
    Known { name: "Vadodara", aliases: &["vadodara", "baroda"], centre: (22.3072, 73.1812) },
    Known { name: "Surat", aliases: &["surat"], centre: (21.1702, 72.8311) },
    Known { name: "Goa", aliases: &["goa", "panaji", "panjim"], centre: (15.4909, 73.8278) },
    Known { name: "Thiruvananthapuram", aliases: &["thiruvananthapuram", "trivandrum"], centre: (8.5241, 76.9366) },
    Known { name: "Visakhapatnam", aliases: &["visakhapatnam", "vizag"], centre: (17.6868, 83.2185) },
    Known { name: "Mysuru", aliases: &["mysuru", "mysore"], centre: (12.2958, 76.6394) },
    Known { name: "Dehradun", aliases: &["dehradun"], centre: (30.3165, 78.0322) },
    Known { name: "Bhopal", aliases: &["bhopal"], centre: (23.2599, 77.4126) },
    Known { name: "Patna", aliases: &["patna"], centre: (25.5941, 85.1376) },
    Known { name: "Guwahati", aliases: &["guwahati"], centre: (26.1445, 91.7362) },
];

/// Longest alias in words; the extractor's lookup window.
pub const MAX_ALIAS_WORDS: usize = 2;

impl City {
    /// A known city by any of its names. `words` is lower-case, single-spaced.
    pub fn from_alias(words: &str) -> Option<Self> {
        KNOWN.iter().find(|k| k.aliases.contains(&words)).map(|k| Self(k.name.to_owned()))
    }

    /// Any city: a known one by alias, otherwise the name as typed, in title
    /// case. `None` when the text cannot be a place name.
    pub fn parse(input: &str) -> Option<Self> {
        let words: Vec<String> = input.split_whitespace().map(|w| w.to_lowercase()).collect();
        let plain = words.join(" ");
        if let Some(city) = Self::from_alias(&plain) {
            return Some(city);
        }
        let name_like = (1..=3).contains(&words.len())
            && plain.chars().count() >= 3
            && plain.chars().all(|c| c.is_alphabetic() || c == ' ');
        name_like.then(|| Self(title_case(&plain)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where the map centres for this city, when it is a known one.
    pub fn centre(&self) -> Option<GeoPoint> {
        KNOWN.iter().find(|k| k.name == self.0).and_then(|k| GeoPoint::new(k.centre.0, k.centre.1))
    }

    /// Every known city with its map centre.
    pub fn known() -> impl Iterator<Item = (City, GeoPoint)> {
        KNOWN.iter().filter_map(|k| GeoPoint::new(k.centre.0, k.centre.1).map(|p| (City(k.name.to_owned()), p)))
    }
}

impl fmt::Display for City {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// "hsr layout" becomes "Hsr Layout".
pub fn title_case(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_of_a_metro_is_one_city() {
        for name in ["blr", "Bangalore", "BENGALURU", "  bengaluru "] {
            assert_eq!(City::parse(name).unwrap().as_str(), "Bengaluru", "{name}");
        }
        assert_eq!(City::parse("gurgaon").unwrap().as_str(), "Gurugram");
        assert_eq!(City::parse("Navi  Mumbai").unwrap().as_str(), "Navi Mumbai");
    }

    #[test]
    fn a_city_from_json_is_canonical_too() {
        let city: City = serde_json::from_str(r#""bangalore""#).unwrap();
        assert_eq!(city.as_str(), "Bengaluru");
        assert_eq!(serde_json::to_string(&city).unwrap(), r#""Bengaluru""#);
        assert!(serde_json::from_str::<City>(r#""<script>""#).is_err());
        for (known, _) in City::known() {
            assert_eq!(City::parse(known.as_str()), Some(known.clone()), "canonical names parse to themselves");
        }
    }

    #[test]
    fn other_cities_are_kept_as_typed() {
        assert_eq!(City::parse("jhansi").unwrap().as_str(), "Jhansi");
        assert_eq!(City::parse("port blair").unwrap().as_str(), "Port Blair");
        assert!(City::parse("").is_none());
        assert!(City::parse("18000").is_none());
        assert!(City::parse("somewhere near the big mall").is_none());
    }

    #[test]
    fn every_known_city_has_a_centre_in_india_and_unique_aliases() {
        assert_eq!(City::known().count(), KNOWN.len());
        let mut seen = std::collections::HashSet::new();
        for known in KNOWN {
            for alias in known.aliases {
                assert!(seen.insert(alias), "alias {alias} is listed twice");
                assert!(alias.split(' ').count() <= MAX_ALIAS_WORDS, "{alias}");
                assert_eq!(*alias, alias.to_lowercase());
            }
        }
    }
}
