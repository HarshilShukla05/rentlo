//! Launch neighbourhoods are fixed public places, independent of profile pins.

use crate::{City, GeoPoint};

#[derive(Debug, Clone, Copy)]
pub struct Area {
    pub city: &'static str,
    pub name: &'static str,
    pub centre: GeoPoint,
    aliases: &'static [&'static str],
}

const fn area(city: &'static str, name: &'static str, lat: f64, lng: f64, aliases: &'static [&'static str]) -> Area {
    Area { city, name, centre: GeoPoint::catalogue_point(lat, lng), aliases }
}

const CATALOGUE: &[Area] = &[
    area(
        "Hyderabad",
        "Financial District",
        17.417,
        78.343,
        &["financial district", "financial fistrict", "nanakramguda"],
    ),
    area("Hyderabad", "Gachibowli", 17.445, 78.351, &["gachibowli", "gachi bowli"]),
    area("Hyderabad", "Kondapur", 17.461, 78.367, &["kondapur"]),
    area("Hyderabad", "Madhapur", 17.451, 78.391, &["madhapur", "hitec city", "hi tech city"]),
    area("Hyderabad", "Kokapet", 17.402, 78.334, &["kokapet"]),
    area("Hyderabad", "Manikonda", 17.403, 78.381, &["manikonda"]),
    area("Hyderabad", "Jubilee Hills", 17.432, 78.408, &["jubilee hills"]),
    area("Hyderabad", "Banjara Hills", 17.416, 78.435, &["banjara hills"]),
    area("Hyderabad", "Kukatpally", 17.486, 78.403, &["kukatpally", "kphb"]),
    area("Hyderabad", "Nallagandla", 17.468, 78.304, &["nallagandla"]),
    area("Bengaluru", "Koramangala", 12.935, 77.626, &["koramangala", "koramangla"]),
    area("Bengaluru", "HSR Layout", 12.911, 77.638, &["hsr layout", "hsr"]),
    area("Bengaluru", "Indiranagar", 12.979, 77.641, &["indiranagar", "indira nagar"]),
    area("Bengaluru", "Whitefield", 12.969, 77.750, &["whitefield"]),
    area("Bengaluru", "Bellandur", 12.928, 77.678, &["bellandur"]),
    area("Bengaluru", "Electronic City", 12.846, 77.663, &["electronic city", "ecity"]),
    area("Bengaluru", "Marathahalli", 12.955, 77.698, &["marathahalli"]),
    area("Bengaluru", "BTM Layout", 12.915, 77.610, &["btm layout", "btm"]),
    area("Bengaluru", "Jayanagar", 12.929, 77.583, &["jayanagar"]),
    area("Mumbai", "Andheri West", 19.136, 72.828, &["andheri west"]),
    area("Mumbai", "Andheri East", 19.116, 72.870, &["andheri east"]),
    area("Mumbai", "Powai", 19.121, 72.905, &["powai"]),
    area("Mumbai", "Bandra", 19.055, 72.835, &["bandra", "bandra west", "bandra east"]),
    area("Mumbai", "Goregaon", 19.165, 72.855, &["goregaon"]),
    area("Mumbai", "Chembur", 19.054, 72.900, &["chembur"]),
    area("Pune", "Hinjawadi", 18.590, 73.737, &["hinjawadi", "hinjewadi"]),
    area("Pune", "Wakad", 18.599, 73.764, &["wakad"]),
    area("Pune", "Baner", 18.560, 73.787, &["baner"]),
    area("Pune", "Kharadi", 18.551, 73.939, &["kharadi"]),
    area("Pune", "Viman Nagar", 18.568, 73.914, &["viman nagar"]),
    area("Pune", "Koregaon Park", 18.537, 73.897, &["koregaon park", "kp"]),
    area("Delhi", "Saket", 28.524, 77.208, &["saket"]),
    area("Delhi", "Lajpat Nagar", 28.569, 77.243, &["lajpat nagar"]),
    area("Delhi", "Dwarka", 28.593, 77.043, &["dwarka"]),
    area("Delhi", "Greater Kailash", 28.540, 77.240, &["greater kailash", "gk"]),
    area("Gurugram", "DLF Phase 3", 28.494, 77.096, &["dlf phase 3", "dlf phase iii"]),
    area("Gurugram", "Golf Course Road", 28.451, 77.100, &["golf course road"]),
    area("Gurugram", "Sohna Road", 28.415, 77.043, &["sohna road"]),
    area("Noida", "Sector 62", 28.623, 77.365, &["sector 62"]),
    area("Noida", "Sector 137", 28.510, 77.410, &["sector 137"]),
    area("Chennai", "Velachery", 12.981, 80.219, &["velachery"]),
    area("Chennai", "Thoraipakkam", 12.940, 80.234, &["thoraipakkam"]),
    area("Chennai", "Sholinganallur", 12.901, 80.227, &["sholinganallur"]),
    area("Chennai", "Porur", 13.037, 80.157, &["porur"]),
    area("Kolkata", "Salt Lake", 22.587, 88.417, &["salt lake", "bidhannagar"]),
    area("Kolkata", "New Town", 22.581, 88.467, &["new town", "newtown", "rajarhat"]),
];

pub fn in_city(city: &City) -> impl Iterator<Item = &'static Area> {
    CATALOGUE.iter().filter(|area| area.city == city.as_str())
}

pub fn find(city: &City, name: &str) -> Option<&'static Area> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    in_city(city).find(|area| area.aliases.contains(&name.as_str()))
}

pub fn mentioned(city: &City, words: &[String]) -> Option<&'static Area> {
    let mut first = None;
    for area in in_city(city) {
        for alias in area.aliases {
            let parts: Vec<_> = alias.split_whitespace().collect();
            let at = words
                .windows(parts.len())
                .position(|window| window.iter().map(String::as_str).eq(parts.iter().copied()));
            if let Some(at) = at {
                let rank = (at, std::cmp::Reverse(parts.len()));
                if first.as_ref().is_none_or(|(previous, _)| rank < *previous) {
                    first = Some((rank, area));
                }
            }
        }
    }
    first.map(|(_, area)| area)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_in_india_and_names_resolve_in_their_city() {
        for area in CATALOGUE {
            let city = City::parse(area.city).unwrap();
            assert_eq!(find(&city, area.name).map(|found| found.name), Some(area.name));
            assert!(GeoPoint::new(area.centre.lat(), area.centre.lng()).is_some());
        }
    }
}
