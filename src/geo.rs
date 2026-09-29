//! Offline reverse geocoding (GeoNames cities with population > 1000).

use std::sync::OnceLock;

use reverse_geocoder::ReverseGeocoder;

pub struct City {
    pub name: String,
    pub region: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

pub fn lookup(lat: f64, lon: f64) -> City {
    static GEOCODER: OnceLock<ReverseGeocoder> = OnceLock::new();
    let record = GEOCODER.get_or_init(ReverseGeocoder::new).search((lat, lon)).record;
    City {
        name: record.name.clone(),
        region: record.admin1.clone(),
        country: record.cc.clone(),
        lat: record.lat,
        lon: record.lon,
    }
}
