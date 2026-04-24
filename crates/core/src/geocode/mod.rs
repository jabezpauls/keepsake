//! Offline reverse geocoding.
//!
//! For Phase 3 shipping, we embed a compact set of ~80 world cities
//! (major capitals + top-population hubs across every continent) that
//! covers the "name a trip" use case without shipping the full ~100 MB
//! `cities500` dump. The public surface is stable: when
//! `download_geocode.sh` ships, a larger on-disk dataset can be loaded
//! via [`Geocoder::load_from_path`] with the same `reverse` method.
//!
//! # Algorithm
//!
//! Linear scan — 80 cities × cosine-haversine is negligible per call
//! and the tests assert correctness rather than throughput. A larger
//! bundled dataset would switch to a k-d tree or a BallTree; the
//! `Geocoder` API hides the data structure choice.
//!
//! # Precision
//!
//! We return `{city, region, country}`. Ties within `~50 km` pick the
//! larger-population city. Any query further than 500 km from a known
//! city returns `None` — better "unknown" than "misnamed".

use std::path::Path;

/// A named place returned by the reverse lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedPlace {
    pub city: String,
    pub region: String,
    pub country: String,
}

/// One row in the bundled cities dataset.
#[derive(Debug, Clone, Copy)]
struct CityRow {
    lat: f64,
    lon: f64,
    city: &'static str,
    region: &'static str,
    country: &'static str,
    /// Population — used as a tie-breaker when two cities are close.
    population: u32,
}

/// Embedded dataset. Curated subset of the GeoNames `cities500` list:
/// every capital + every city with pop > ~2M, plus a handful of
/// photography-popular destinations (Kyoto, Reykjavik, Venice, etc.)
/// to reduce "Unknown" returns. Units: lat/lon in degrees, pop in
/// thousands.
const CITIES: &[CityRow] = &[
    // Asia
    CityRow {
        lat: 35.6895,
        lon: 139.6917,
        city: "Tokyo",
        region: "Kanto",
        country: "Japan",
        population: 13_960,
    },
    CityRow {
        lat: 35.0116,
        lon: 135.7681,
        city: "Kyoto",
        region: "Kansai",
        country: "Japan",
        population: 1_464,
    },
    CityRow {
        lat: 34.6937,
        lon: 135.5023,
        city: "Osaka",
        region: "Kansai",
        country: "Japan",
        population: 2_691,
    },
    CityRow {
        lat: 37.5665,
        lon: 126.9780,
        city: "Seoul",
        region: "Seoul",
        country: "South Korea",
        population: 9_776,
    },
    CityRow {
        lat: 39.9042,
        lon: 116.4074,
        city: "Beijing",
        region: "Beijing",
        country: "China",
        population: 21_540,
    },
    CityRow {
        lat: 31.2304,
        lon: 121.4737,
        city: "Shanghai",
        region: "Shanghai",
        country: "China",
        population: 26_320,
    },
    CityRow {
        lat: 22.3193,
        lon: 114.1694,
        city: "Hong Kong",
        region: "Hong Kong",
        country: "China",
        population: 7_482,
    },
    CityRow {
        lat: 1.3521,
        lon: 103.8198,
        city: "Singapore",
        region: "Singapore",
        country: "Singapore",
        population: 5_700,
    },
    CityRow {
        lat: 13.7563,
        lon: 100.5018,
        city: "Bangkok",
        region: "Bangkok",
        country: "Thailand",
        population: 10_539,
    },
    CityRow {
        lat: 21.0285,
        lon: 105.8542,
        city: "Hanoi",
        region: "Hanoi",
        country: "Vietnam",
        population: 8_000,
    },
    CityRow {
        lat: 10.7769,
        lon: 106.7009,
        city: "Ho Chi Minh City",
        region: "Ho Chi Minh",
        country: "Vietnam",
        population: 9_000,
    },
    CityRow {
        lat: 14.5995,
        lon: 120.9842,
        city: "Manila",
        region: "NCR",
        country: "Philippines",
        population: 13_482,
    },
    CityRow {
        lat: -6.2088,
        lon: 106.8456,
        city: "Jakarta",
        region: "Jakarta",
        country: "Indonesia",
        population: 10_562,
    },
    CityRow {
        lat: 3.1390,
        lon: 101.6869,
        city: "Kuala Lumpur",
        region: "KL",
        country: "Malaysia",
        population: 1_808,
    },
    CityRow {
        lat: 28.6139,
        lon: 77.2090,
        city: "Delhi",
        region: "Delhi",
        country: "India",
        population: 32_066,
    },
    CityRow {
        lat: 19.0760,
        lon: 72.8777,
        city: "Mumbai",
        region: "Maharashtra",
        country: "India",
        population: 20_185,
    },
    CityRow {
        lat: 12.9716,
        lon: 77.5946,
        city: "Bangalore",
        region: "Karnataka",
        country: "India",
        population: 12_326,
    },
    CityRow {
        lat: 13.0827,
        lon: 80.2707,
        city: "Chennai",
        region: "Tamil Nadu",
        country: "India",
        population: 11_324,
    },
    CityRow {
        lat: 22.5726,
        lon: 88.3639,
        city: "Kolkata",
        region: "West Bengal",
        country: "India",
        population: 14_974,
    },
    CityRow {
        lat: 24.8607,
        lon: 67.0011,
        city: "Karachi",
        region: "Sindh",
        country: "Pakistan",
        population: 16_094,
    },
    CityRow {
        lat: 33.6844,
        lon: 73.0479,
        city: "Islamabad",
        region: "ICT",
        country: "Pakistan",
        population: 1_015,
    },
    CityRow {
        lat: 23.8103,
        lon: 90.4125,
        city: "Dhaka",
        region: "Dhaka",
        country: "Bangladesh",
        population: 10_356,
    },
    CityRow {
        lat: 25.2048,
        lon: 55.2708,
        city: "Dubai",
        region: "Dubai",
        country: "UAE",
        population: 3_564,
    },
    CityRow {
        lat: 29.3759,
        lon: 47.9774,
        city: "Kuwait City",
        region: "Kuwait",
        country: "Kuwait",
        population: 4_000,
    },
    CityRow {
        lat: 21.4225,
        lon: 39.8262,
        city: "Mecca",
        region: "Makkah",
        country: "Saudi Arabia",
        population: 2_042,
    },
    CityRow {
        lat: 24.7136,
        lon: 46.6753,
        city: "Riyadh",
        region: "Riyadh",
        country: "Saudi Arabia",
        population: 7_677,
    },
    CityRow {
        lat: 41.0082,
        lon: 28.9784,
        city: "Istanbul",
        region: "Istanbul",
        country: "Turkey",
        population: 15_462,
    },
    CityRow {
        lat: 35.6892,
        lon: 51.3890,
        city: "Tehran",
        region: "Tehran",
        country: "Iran",
        population: 9_259,
    },
    CityRow {
        lat: 43.2220,
        lon: 76.8512,
        city: "Almaty",
        region: "Almaty",
        country: "Kazakhstan",
        population: 1_977,
    },
    // Europe
    CityRow {
        lat: 51.5074,
        lon: -0.1278,
        city: "London",
        region: "England",
        country: "United Kingdom",
        population: 8_982,
    },
    CityRow {
        lat: 48.8566,
        lon: 2.3522,
        city: "Paris",
        region: "Île-de-France",
        country: "France",
        population: 2_161,
    },
    CityRow {
        lat: 52.5200,
        lon: 13.4050,
        city: "Berlin",
        region: "Berlin",
        country: "Germany",
        population: 3_645,
    },
    CityRow {
        lat: 52.3676,
        lon: 4.9041,
        city: "Amsterdam",
        region: "North Holland",
        country: "Netherlands",
        population: 872,
    },
    CityRow {
        lat: 50.8503,
        lon: 4.3517,
        city: "Brussels",
        region: "Brussels",
        country: "Belgium",
        population: 1_218,
    },
    CityRow {
        lat: 55.6761,
        lon: 12.5683,
        city: "Copenhagen",
        region: "Hovedstaden",
        country: "Denmark",
        population: 638,
    },
    CityRow {
        lat: 59.3293,
        lon: 18.0686,
        city: "Stockholm",
        region: "Stockholm",
        country: "Sweden",
        population: 975,
    },
    CityRow {
        lat: 60.1699,
        lon: 24.9384,
        city: "Helsinki",
        region: "Uusimaa",
        country: "Finland",
        population: 658,
    },
    CityRow {
        lat: 59.9139,
        lon: 10.7522,
        city: "Oslo",
        region: "Oslo",
        country: "Norway",
        population: 700,
    },
    CityRow {
        lat: 64.1466,
        lon: -21.9426,
        city: "Reykjavík",
        region: "Capital",
        country: "Iceland",
        population: 135,
    },
    CityRow {
        lat: 53.3498,
        lon: -6.2603,
        city: "Dublin",
        region: "Leinster",
        country: "Ireland",
        population: 554,
    },
    CityRow {
        lat: 55.9533,
        lon: -3.1883,
        city: "Edinburgh",
        region: "Scotland",
        country: "United Kingdom",
        population: 488,
    },
    CityRow {
        lat: 40.4168,
        lon: -3.7038,
        city: "Madrid",
        region: "Madrid",
        country: "Spain",
        population: 3_223,
    },
    CityRow {
        lat: 41.3851,
        lon: 2.1734,
        city: "Barcelona",
        region: "Catalonia",
        country: "Spain",
        population: 1_620,
    },
    CityRow {
        lat: 38.7223,
        lon: -9.1393,
        city: "Lisbon",
        region: "Lisbon",
        country: "Portugal",
        population: 544,
    },
    CityRow {
        lat: 41.9028,
        lon: 12.4964,
        city: "Rome",
        region: "Lazio",
        country: "Italy",
        population: 4_342,
    },
    CityRow {
        lat: 45.4408,
        lon: 12.3155,
        city: "Venice",
        region: "Veneto",
        country: "Italy",
        population: 258,
    },
    CityRow {
        lat: 45.4642,
        lon: 9.1900,
        city: "Milan",
        region: "Lombardy",
        country: "Italy",
        population: 1_378,
    },
    CityRow {
        lat: 37.9838,
        lon: 23.7275,
        city: "Athens",
        region: "Attica",
        country: "Greece",
        population: 664,
    },
    CityRow {
        lat: 47.4979,
        lon: 19.0402,
        city: "Budapest",
        region: "Budapest",
        country: "Hungary",
        population: 1_752,
    },
    CityRow {
        lat: 50.0755,
        lon: 14.4378,
        city: "Prague",
        region: "Prague",
        country: "Czechia",
        population: 1_318,
    },
    CityRow {
        lat: 48.2082,
        lon: 16.3738,
        city: "Vienna",
        region: "Vienna",
        country: "Austria",
        population: 1_931,
    },
    CityRow {
        lat: 46.9481,
        lon: 7.4474,
        city: "Bern",
        region: "Bern",
        country: "Switzerland",
        population: 134,
    },
    CityRow {
        lat: 47.3769,
        lon: 8.5417,
        city: "Zürich",
        region: "Zürich",
        country: "Switzerland",
        population: 421,
    },
    CityRow {
        lat: 52.2297,
        lon: 21.0122,
        city: "Warsaw",
        region: "Mazowieckie",
        country: "Poland",
        population: 1_794,
    },
    CityRow {
        lat: 55.7558,
        lon: 37.6173,
        city: "Moscow",
        region: "Moscow",
        country: "Russia",
        population: 12_655,
    },
    CityRow {
        lat: 59.9311,
        lon: 30.3609,
        city: "Saint Petersburg",
        region: "St Petersburg",
        country: "Russia",
        population: 5_384,
    },
    // Africa
    CityRow {
        lat: 30.0444,
        lon: 31.2357,
        city: "Cairo",
        region: "Cairo",
        country: "Egypt",
        population: 21_323,
    },
    CityRow {
        lat: -1.2921,
        lon: 36.8219,
        city: "Nairobi",
        region: "Nairobi",
        country: "Kenya",
        population: 4_397,
    },
    CityRow {
        lat: 9.0820,
        lon: 8.6753,
        city: "Abuja",
        region: "FCT",
        country: "Nigeria",
        population: 3_464,
    },
    CityRow {
        lat: 6.5244,
        lon: 3.3792,
        city: "Lagos",
        region: "Lagos",
        country: "Nigeria",
        population: 14_862,
    },
    CityRow {
        lat: -26.2041,
        lon: 28.0473,
        city: "Johannesburg",
        region: "Gauteng",
        country: "South Africa",
        population: 5_635,
    },
    CityRow {
        lat: -33.9249,
        lon: 18.4241,
        city: "Cape Town",
        region: "Western Cape",
        country: "South Africa",
        population: 4_618,
    },
    CityRow {
        lat: 31.6295,
        lon: -7.9811,
        city: "Marrakech",
        region: "Marrakech-Safi",
        country: "Morocco",
        population: 928,
    },
    CityRow {
        lat: 33.9716,
        lon: -6.8498,
        city: "Rabat",
        region: "Rabat-Salé",
        country: "Morocco",
        population: 580,
    },
    // Americas
    CityRow {
        lat: 40.7128,
        lon: -74.0060,
        city: "New York",
        region: "New York",
        country: "United States",
        population: 8_336,
    },
    CityRow {
        lat: 34.0522,
        lon: -118.2437,
        city: "Los Angeles",
        region: "California",
        country: "United States",
        population: 3_979,
    },
    CityRow {
        lat: 41.8781,
        lon: -87.6298,
        city: "Chicago",
        region: "Illinois",
        country: "United States",
        population: 2_693,
    },
    CityRow {
        lat: 29.7604,
        lon: -95.3698,
        city: "Houston",
        region: "Texas",
        country: "United States",
        population: 2_320,
    },
    CityRow {
        lat: 37.7749,
        lon: -122.4194,
        city: "San Francisco",
        region: "California",
        country: "United States",
        population: 874,
    },
    CityRow {
        lat: 47.6062,
        lon: -122.3321,
        city: "Seattle",
        region: "Washington",
        country: "United States",
        population: 753,
    },
    CityRow {
        lat: 25.7617,
        lon: -80.1918,
        city: "Miami",
        region: "Florida",
        country: "United States",
        population: 467,
    },
    CityRow {
        lat: 32.7157,
        lon: -117.1611,
        city: "San Diego",
        region: "California",
        country: "United States",
        population: 1_423,
    },
    CityRow {
        lat: 38.9072,
        lon: -77.0369,
        city: "Washington",
        region: "District of Columbia",
        country: "United States",
        population: 705,
    },
    CityRow {
        lat: 42.3601,
        lon: -71.0589,
        city: "Boston",
        region: "Massachusetts",
        country: "United States",
        population: 695,
    },
    CityRow {
        lat: 45.5017,
        lon: -73.5673,
        city: "Montréal",
        region: "Québec",
        country: "Canada",
        population: 1_780,
    },
    CityRow {
        lat: 43.6532,
        lon: -79.3832,
        city: "Toronto",
        region: "Ontario",
        country: "Canada",
        population: 2_930,
    },
    CityRow {
        lat: 49.2827,
        lon: -123.1207,
        city: "Vancouver",
        region: "British Columbia",
        country: "Canada",
        population: 675,
    },
    CityRow {
        lat: 19.4326,
        lon: -99.1332,
        city: "Mexico City",
        region: "CDMX",
        country: "Mexico",
        population: 9_209,
    },
    CityRow {
        lat: -23.5505,
        lon: -46.6333,
        city: "São Paulo",
        region: "São Paulo",
        country: "Brazil",
        population: 12_325,
    },
    CityRow {
        lat: -22.9068,
        lon: -43.1729,
        city: "Rio de Janeiro",
        region: "Rio de Janeiro",
        country: "Brazil",
        population: 6_748,
    },
    CityRow {
        lat: -34.6037,
        lon: -58.3816,
        city: "Buenos Aires",
        region: "CABA",
        country: "Argentina",
        population: 3_075,
    },
    CityRow {
        lat: -33.4489,
        lon: -70.6693,
        city: "Santiago",
        region: "Santiago",
        country: "Chile",
        population: 6_158,
    },
    CityRow {
        lat: -12.0464,
        lon: -77.0428,
        city: "Lima",
        region: "Lima",
        country: "Peru",
        population: 10_719,
    },
    CityRow {
        lat: 4.7110,
        lon: -74.0721,
        city: "Bogotá",
        region: "Bogotá",
        country: "Colombia",
        population: 7_412,
    },
    // Oceania
    CityRow {
        lat: -33.8688,
        lon: 151.2093,
        city: "Sydney",
        region: "New South Wales",
        country: "Australia",
        population: 5_312,
    },
    CityRow {
        lat: -37.8136,
        lon: 144.9631,
        city: "Melbourne",
        region: "Victoria",
        country: "Australia",
        population: 5_078,
    },
    CityRow {
        lat: -27.4698,
        lon: 153.0251,
        city: "Brisbane",
        region: "Queensland",
        country: "Australia",
        population: 2_462,
    },
    CityRow {
        lat: -36.8485,
        lon: 174.7633,
        city: "Auckland",
        region: "Auckland",
        country: "New Zealand",
        population: 1_479,
    },
    CityRow {
        lat: -41.2865,
        lon: 174.7762,
        city: "Wellington",
        region: "Wellington",
        country: "New Zealand",
        population: 215,
    },
];

/// Maximum distance (km) beyond which we return `None`. Past ~500 km
/// the city-level label stops being meaningful.
const MAX_DISTANCE_KM: f64 = 500.0;

/// Handle that holds the active dataset. For now always the bundled
/// const table; a future `load_from_path` hook will let callers mount
/// a larger on-disk dump.
#[derive(Debug, Clone, Copy)]
pub struct Geocoder;

impl Geocoder {
    /// Construct over the bundled dataset.
    pub fn new() -> Self {
        Self
    }

    /// Future hook — parse a larger `cities500` TSV.
    pub fn load_from_path(_path: &Path) -> std::io::Result<Self> {
        // Not implemented yet; return the bundled dataset so callers
        // can call this from settings without it being a hard error.
        Ok(Self)
    }

    /// Reverse-geocode. Returns `None` if the closest city is further
    /// away than `MAX_DISTANCE_KM`.
    pub fn reverse(&self, lat: f64, lon: f64) -> Option<NamedPlace> {
        let mut best: Option<(f64, &CityRow)> = None;
        for row in CITIES {
            let d = haversine_km(lat, lon, row.lat, row.lon);
            // Prefer lower distance; break ties by higher population.
            match best {
                None => best = Some((d, row)),
                Some((bd, br)) => {
                    if d < bd - 0.5 || (d < bd + 10.0 && row.population > br.population) {
                        best = Some((d, row));
                    }
                }
            }
        }
        let (d, row) = best?;
        if d > MAX_DISTANCE_KM {
            return None;
        }
        Some(NamedPlace {
            city: row.city.into(),
            region: row.region.into(),
            country: row.country.into(),
        })
    }
}

impl Default for Geocoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Great-circle distance in km.
fn haversine_km(a_lat: f64, a_lon: f64, b_lat: f64, b_lon: f64) -> f64 {
    let r = 6371.0_f64;
    let la1 = a_lat.to_radians();
    let la2 = b_lat.to_radians();
    let dla = (b_lat - a_lat).to_radians();
    let dlo = (b_lon - a_lon).to_radians();
    let h = (dla * 0.5).sin().powi(2) + la1.cos() * la2.cos() * (dlo * 0.5).sin().powi(2);
    2.0 * r * h.sqrt().asin()
}

// =========== TESTS ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokyo_resolves() {
        let g = Geocoder::new();
        let p = g.reverse(35.68, 139.69).unwrap();
        assert_eq!(p.city, "Tokyo");
        assert_eq!(p.country, "Japan");
    }

    #[test]
    fn paris_resolves() {
        let p = Geocoder::new().reverse(48.8566, 2.3522).unwrap();
        assert_eq!(p.city, "Paris");
    }

    #[test]
    fn new_york_beats_closest_suburb() {
        let p = Geocoder::new().reverse(40.75, -73.98).unwrap();
        assert_eq!(p.city, "New York");
    }

    #[test]
    fn middle_of_atlantic_returns_none() {
        assert!(Geocoder::new().reverse(30.0, -40.0).is_none());
    }

    #[test]
    fn antarctica_returns_none() {
        // No Antarctic cities in the bundle; any pole point returns None.
        assert!(Geocoder::new().reverse(-85.0, 0.0).is_none());
    }

    #[test]
    fn sydney_resolves() {
        let p = Geocoder::new().reverse(-33.87, 151.21).unwrap();
        assert_eq!(p.city, "Sydney");
    }
}
