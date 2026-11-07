//! Locations, as the geospatial commands store them: a longitude and latitude are turned
//! into a 52-bit geohash, which is the location's score in a sorted set. Each coordinate
//! is scaled to a 26-bit grid number, and the two numbers' bits are interleaved, the
//! latitude's in the even positions and the longitude's in the odd ones. Decoding gives
//! the centre of the grid cell, which is close to the original point but not exact.

/// The latitudes the Web Mercator projection can show, which Redis limits locations to.
pub const MIN_LATITUDE: f64 = -85.05112878;
pub const MAX_LATITUDE: f64 = 85.05112878;
pub const MIN_LONGITUDE: f64 = -180.0;
pub const MAX_LONGITUDE: f64 = 180.0;
const LATITUDE_RANGE: f64 = MAX_LATITUDE - MIN_LATITUDE;
const LONGITUDE_RANGE: f64 = MAX_LONGITUDE - MIN_LONGITUDE;
/// How many grid cells each coordinate is divided into: 2^26.
const CELLS: f64 = (1u64 << 26) as f64;
/// The Earth's radius in meters, as Redis takes it.
const EARTH_RADIUS: f64 = 6372797.560856;

/// Whether a location can be stored.
pub fn is_valid(longitude: f64, latitude: f64) -> bool {
    (MIN_LONGITUDE..=MAX_LONGITUDE).contains(&longitude)
        && (MIN_LATITUDE..=MAX_LATITUDE).contains(&latitude)
}

/// The geohash of a location, which has to be valid.
pub fn encode(longitude: f64, latitude: f64) -> u64 {
    // Truncated to the grid cell the location is in.
    let lat = (CELLS * (latitude - MIN_LATITUDE) / LATITUDE_RANGE) as u32;
    let lon = (CELLS * (longitude - MIN_LONGITUDE) / LONGITUDE_RANGE) as u32;
    spread(lat) | spread(lon) << 1
}

/// The location a geohash stands for: the centre of its grid cell, as (longitude, latitude).
pub fn decode(hash: u64) -> (f64, f64) {
    let lat = f64::from(compact(hash));
    let lon = f64::from(compact(hash >> 1));
    let lat_min = MIN_LATITUDE + LATITUDE_RANGE * (lat / CELLS);
    let lat_max = MIN_LATITUDE + LATITUDE_RANGE * ((lat + 1.0) / CELLS);
    let lon_min = MIN_LONGITUDE + LONGITUDE_RANGE * (lon / CELLS);
    let lon_max = MIN_LONGITUDE + LONGITUDE_RANGE * ((lon + 1.0) / CELLS);
    let longitude = ((lon_min + lon_max) / 2.0).clamp(MIN_LONGITUDE, MAX_LONGITUDE);
    let latitude = ((lat_min + lat_max) / 2.0).clamp(MIN_LATITUDE, MAX_LATITUDE);
    (longitude, latitude)
}

/// Spreads out the bits of `v` into the even positions of a 64-bit number.
fn spread(v: u32) -> u64 {
    let mut v = u64::from(v);
    v = (v | v << 16) & 0x0000_FFFF_0000_FFFF;
    v = (v | v << 8) & 0x00FF_00FF_00FF_00FF;
    v = (v | v << 4) & 0x0F0F_0F0F_0F0F_0F0F;
    v = (v | v << 2) & 0x3333_3333_3333_3333;
    v = (v | v << 1) & 0x5555_5555_5555_5555;
    v
}

/// Gathers the bits in the even positions of `v` back together: the reverse of `spread`.
fn compact(v: u64) -> u32 {
    let mut v = v & 0x5555_5555_5555_5555;
    v = (v | v >> 1) & 0x3333_3333_3333_3333;
    v = (v | v >> 2) & 0x0F0F_0F0F_0F0F_0F0F;
    v = (v | v >> 4) & 0x00FF_00FF_00FF_00FF;
    v = (v | v >> 8) & 0x0000_FFFF_0000_FFFF;
    v = (v | v >> 16) & 0x0000_0000_FFFF_FFFF;
    v as u32
}

/// The distance in meters between two locations, given as (longitude, latitude), along
/// the Earth's surface: the haversine formula, worked out as Redis does.
pub fn distance(from: (f64, f64), to: (f64, f64)) -> f64 {
    let (lon1, lon2) = (from.0.to_radians(), to.0.to_radians());
    let v = ((lon2 - lon1) / 2.0).sin();
    // On the same meridian, the distance is just the difference in latitude.
    if v == 0.0 {
        return EARTH_RADIUS * (to.1.to_radians() - from.1.to_radians()).abs();
    }
    let (lat1, lat2) = (from.1.to_radians(), to.1.to_radians());
    let u = ((lat2 - lat1) / 2.0).sin();
    let a = u * u + lat1.cos() * lat2.cos() * v * v;
    2.0 * EARTH_RADIUS * a.sqrt().asin()
}

/// How many meters there are in a unit of distance: m, km, mi or ft, in any case.
pub fn unit_in_meters(unit: &[u8]) -> Option<f64> {
    match unit.to_ascii_lowercase().as_slice() {
        b"m" => Some(1.0),
        b"km" => Some(1000.0),
        b"mi" => Some(1609.34),
        b"ft" => Some(0.3048),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Worked out with the reference implementation in
    // github.com/codecrafters-io/redis-geocoding-algorithm.
    const PLACES: &[(&str, f64, f64, u64)] = &[
        ("Paris", 2.2944692, 48.8584625, 3663832614298053),
        ("London", -0.127758, 51.507351, 2163557714754256),
        ("Munich", 11.5030378, 48.164271, 3672376881541190),
        ("Bangkok", 100.5252, 13.722, 3962257306574459),
        ("Sydney", 151.2093, -33.8688, 3252046221964352),
        ("New York", -74.006, 40.7128, 1791873974549446),
    ];

    #[test]
    fn encodes_locations() {
        for &(name, longitude, latitude, hash) in PLACES {
            assert_eq!(encode(longitude, latitude), hash, "{name}");
        }
    }

    #[test]
    fn decodes_to_the_middle_of_the_grid_cell() {
        assert_eq!(decode(3663832614298053), (2.294471561908722, 48.85846255040141));
        assert_eq!(decode(3876464048901851), (49.12499874830246, 72.99100027813947));
        for &(name, longitude, latitude, hash) in PLACES {
            let (lon, lat) = decode(hash);
            assert!((lon - longitude).abs() < 1e-5 && (lat - latitude).abs() < 1e-5, "{name}");
        }
    }

    #[test]
    fn measures_distances_as_redis_does() {
        let munich = decode(encode(11.5030378, 48.164271));
        let paris = decode(encode(2.2944692, 48.8584625));
        assert_eq!(format!("{:.4}", distance(munich, paris)), "682477.7582");
        assert_eq!(distance(paris, paris), 0.0);
        // Along a meridian.
        let d = distance((10.0, 0.0), (10.0, 1.0));
        assert!((d - 111226.3).abs() < 0.1, "{d}");
    }

    #[test]
    fn checks_coordinates() {
        assert!(is_valid(180.0, 85.05112878));
        assert!(is_valid(-180.0, -85.05112878));
        assert!(!is_valid(180.1, 0.0));
        assert!(!is_valid(0.0, 85.06));
        assert!(!is_valid(f64::NAN, 0.0));
    }

    #[test]
    fn converts_units() {
        assert_eq!(unit_in_meters(b"m"), Some(1.0));
        assert_eq!(unit_in_meters(b"KM"), Some(1000.0));
        assert_eq!(unit_in_meters(b"mi"), Some(1609.34));
        assert_eq!(unit_in_meters(b"ft"), Some(0.3048));
        assert_eq!(unit_in_meters(b"yd"), None);
    }
}
