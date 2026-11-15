//! Points on the map, distance, and the blur that keeps a home private.

use crate::ids::ProfileId;
use serde::{Deserialize, Serialize};

/// A point inside India. The bounds reject typos and swapped coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    lat: f64,
    lng: f64,
}

impl GeoPoint {
    const LAT: std::ops::RangeInclusive<f64> = 6.0..=37.5;
    const LNG: std::ops::RangeInclusive<f64> = 68.0..=97.5;

    pub fn new(lat: f64, lng: f64) -> Option<Self> {
        (Self::LAT.contains(&lat) && Self::LNG.contains(&lng)).then_some(Self { lat, lng })
    }

    pub const fn lat(self) -> f64 {
        self.lat
    }

    pub const fn lng(self) -> f64 {
        self.lng
    }

    /// Great-circle distance in kilometres (haversine).
    pub fn km_to(self, other: GeoPoint) -> f64 {
        const EARTH_RADIUS_KM: f64 = 6371.0;
        let (lat1, lat2) = (self.lat.to_radians(), other.lat.to_radians());
        let d_lat = lat2 - lat1;
        let d_lng = (other.lng - self.lng).to_radians();
        let a = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lng / 2.0).sin().powi(2);
        2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
    }
}

/// Side of a blur cell in degrees: about 1.1 km north-south.
pub const BLUR_CELL_DEG: f64 = 0.01;

/// The only form of a profile's location that may leave the server.
///
/// The exact point is replaced by a spot inside its grid cell. The spot
/// depends on the cell and the profile id alone, so it never moves between
/// requests (many samples cannot be averaged back to the home) and two
/// profiles in one cell do not sit on top of each other.
pub fn blur(point: GeoPoint, profile: ProfileId) -> GeoPoint {
    let (frac_lat, frac_lng) = cell_offset(profile);
    let snap = |value: f64, frac: f64| ((value / BLUR_CELL_DEG).floor() + frac) * BLUR_CELL_DEG;
    GeoPoint { lat: snap(point.lat, frac_lat), lng: snap(point.lng, frac_lng) }
}

/// Two fractions in [0.15, 0.85], fixed for a profile. FNV-1a, written out
/// so the result can never change with a compiler or library version.
fn cell_offset(profile: ProfileId) -> (f64, f64) {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in profile.as_uuid().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let frac = |bits: u64| 0.15 + 0.70 * ((bits & 0xFFFF) as f64 / 65535.0);
    (frac(hash), frac(hash >> 16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn point(lat: f64, lng: f64) -> GeoPoint {
        GeoPoint::new(lat, lng).unwrap()
    }

    #[test]
    fn points_outside_india_are_rejected() {
        assert!(GeoPoint::new(12.97, 77.59).is_some());
        assert!(GeoPoint::new(77.59, 12.97).is_none(), "swapped");
        assert!(GeoPoint::new(51.5, -0.12).is_none());
        assert!(GeoPoint::new(f64::NAN, 77.0).is_none());
    }

    #[test]
    fn distance_matches_known_places() {
        // Koramangala to Indiranagar is about 5 km; Bengaluru to Mumbai about 845 km.
        let koramangala = point(12.9352, 77.6245);
        let indiranagar = point(12.9784, 77.6408);
        let mumbai = point(19.0760, 72.8777);
        assert!((koramangala.km_to(indiranagar) - 5.1).abs() < 0.5);
        assert!((koramangala.km_to(mumbai) - 842.0).abs() < 15.0);
        assert_eq!(koramangala.km_to(koramangala), 0.0);
    }

    #[test]
    fn blur_stays_in_the_cell_and_never_moves() {
        let home = point(12.93527, 77.62451);
        let profile = ProfileId(Uuid::from_u128(42));
        let shown = blur(home, profile);
        assert_eq!(shown, blur(home, profile));
        assert_eq!((shown.lat() / BLUR_CELL_DEG).floor(), (home.lat() / BLUR_CELL_DEG).floor());
        assert_eq!((shown.lng() / BLUR_CELL_DEG).floor(), (home.lng() / BLUR_CELL_DEG).floor());
        // Anywhere in the cell blurs to the same spot: the output carries no
        // more than the cell.
        assert_eq!(shown, blur(point(12.93999, 77.62001), profile));
    }

    #[test]
    fn two_profiles_in_one_cell_show_apart() {
        let home = point(12.9352, 77.6245);
        let a = blur(home, ProfileId(Uuid::from_u128(1)));
        let b = blur(home, ProfileId(Uuid::from_u128(2)));
        assert_ne!(a, b);
    }
}
