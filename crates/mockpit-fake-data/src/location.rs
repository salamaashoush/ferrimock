//! Location and address generators

use fake::Fake;
use fake::faker::address::en::*;

/// Generate a random street name
pub fn fake_street() -> String {
    StreetName().fake()
}

/// Generate a random street address
pub fn fake_street_address() -> String {
    StreetSuffix().fake()
}

/// Generate a random city name
pub fn fake_city() -> String {
    CityName().fake()
}

/// Generate a random state name
pub fn fake_state() -> String {
    StateName().fake()
}

/// Generate a random state abbreviation
pub fn fake_state_abbr() -> String {
    StateAbbr().fake()
}

/// Generate a random ZIP code
pub fn fake_zip() -> String {
    ZipCode().fake()
}

/// Generate a random postal code (alias for fake_zip)
pub fn fake_postal_code() -> String {
    fake_zip()
}

/// Generate a random country name
pub fn fake_country() -> String {
    CountryName().fake()
}

/// Generate a random country code (US, GB, etc.)
pub fn fake_country_code() -> String {
    CountryCode().fake()
}

/// Generate a random latitude
pub fn fake_latitude() -> String {
    Latitude().fake::<f64>().to_string()
}

/// Generate a random longitude
pub fn fake_longitude() -> String {
    Longitude().fake::<f64>().to_string()
}

/// Generate a random building number
pub fn fake_building_number() -> String {
    BuildingNumber().fake()
}

/// Generate a random secondary address (Apt 4, Suite 200, etc.)
pub fn fake_secondary_address() -> String {
    SecondaryAddress().fake()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_street() {
        let street = fake_street();
        assert!(!street.is_empty());
    }

    #[test]
    fn test_fake_city() {
        let city = fake_city();
        assert!(!city.is_empty());
    }

    #[test]
    fn test_fake_state() {
        let state = fake_state();
        assert!(!state.is_empty());
    }

    #[test]
    fn test_fake_state_abbr() {
        let abbr = fake_state_abbr();
        assert!(!abbr.is_empty());
        assert!(abbr.len() <= 3);
    }

    #[test]
    fn test_fake_zip() {
        let zip = fake_zip();
        assert!(!zip.is_empty());
    }

    #[test]
    fn test_fake_country() {
        let country = fake_country();
        assert!(!country.is_empty());
    }

    #[test]
    fn test_fake_country_code() {
        let code = fake_country_code();
        assert!(!code.is_empty());
        assert!(code.len() <= 3);
    }

    #[test]
    fn test_fake_latitude() {
        let lat = fake_latitude();
        assert!(!lat.is_empty());
        let lat_f: f64 = lat.parse().expect("latitude should parse to f64");
        assert!((-90.0..=90.0).contains(&lat_f));
    }

    #[test]
    fn test_fake_longitude() {
        let lon = fake_longitude();
        assert!(!lon.is_empty());
        let _lon_f: f64 = lon.parse().expect("longitude should parse to f64");
    }
}
