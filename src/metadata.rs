//! What the scan reads from a photo's EXIF data: when and where it was taken.

/// When the photo was taken, as "YYYY-MM-DD HH:MM:SS" in the camera's time: the original
/// date, else the digitized one, else the file's EXIF date. Placeholder dates (year 0, say)
/// count as none.
pub(crate) fn taken(exif: &exif::Exif) -> Option<String> {
    [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime].into_iter().find_map(|tag| {
        let field = exif.get_field(tag, exif::In::PRIMARY)?;
        let exif::Value::Ascii(ref parts) = field.value else { return None };
        let dt = exif::DateTime::from_ascii(parts.first()?).ok()?;
        if dt.year < 1900 || dt.month == 0 || dt.day == 0 {
            return None;
        }
        Some(format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second))
    })
}

/// Where the photo was taken, as latitude and longitude in degrees (south and west
/// negative). None without a usable position.
pub(crate) fn position(exif: &exif::Exif) -> Option<(f64, f64)> {
    let coord = |value_tag, ref_tag, negative: u8| -> Option<f64> {
        let field = exif.get_field(value_tag, exif::In::PRIMARY)?;
        let exif::Value::Rational(ref v) = field.value else { return None };
        if v.len() < 3 || v.iter().any(|r| r.denom == 0) {
            return None;
        }
        let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let sign = match exif.get_field(ref_tag, exif::In::PRIMARY).map(|f| &f.value) {
            Some(exif::Value::Ascii(r)) if r.first().and_then(|s| s.first()) == Some(&negative) => -1.0,
            _ => 1.0,
        };
        Some(deg * sign)
    };
    let lat = coord(exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef, b'S')?;
    let lon = coord(exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef, b'W')?;
    // 0,0 is what many cameras write when they have no fix.
    let valid = lat.abs() <= 90.0 && lon.abs() <= 180.0 && (lat.abs() > 1e-6 || lon.abs() > 1e-6);
    valid.then_some((lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::{Field, In, Rational, Tag, Value};

    fn exif_of(fields: &[Field]) -> exif::Exif {
        let mut writer = exif::experimental::Writer::new();
        for f in fields {
            writer.push_field(f);
        }
        let mut tiff = std::io::Cursor::new(Vec::new());
        writer.write(&mut tiff, false).unwrap();
        exif::Reader::new().read_raw(tiff.into_inner()).unwrap()
    }

    fn ascii(tag: Tag, text: &str) -> Field {
        Field { tag, ifd_num: In::PRIMARY, value: Value::Ascii(vec![text.into()]) }
    }

    fn degrees(tag: Tag, d: u32, m: u32, s: u32, denom: u32) -> Field {
        let r = |num, denom| Rational { num, denom };
        Field { tag, ifd_num: In::PRIMARY, value: Value::Rational(vec![r(d, 1), r(m, 1), r(s, denom)]) }
    }

    #[test]
    fn the_original_date_comes_first() {
        let exif = exif_of(&[
            ascii(Tag::DateTime, "2024:01:02 03:04:05"),
            ascii(Tag::DateTimeOriginal, "2021:06:15 10:30:00"),
        ]);
        assert_eq!(taken(&exif).as_deref(), Some("2021-06-15 10:30:00"));
        // Without it, the file's EXIF date.
        let exif = exif_of(&[ascii(Tag::DateTime, "2024:01:02 03:04:05")]);
        assert_eq!(taken(&exif).as_deref(), Some("2024-01-02 03:04:05"));
    }

    #[test]
    fn placeholder_dates_are_no_date() {
        for text in ["0000:00:00 00:00:00", "2021:00:15 10:30:00", "    :  :     :  :  ", "yesterday"] {
            assert_eq!(taken(&exif_of(&[ascii(Tag::DateTimeOriginal, text)])), None, "{text}");
        }
    }

    #[test]
    fn south_and_west_are_negative() {
        let exif = exif_of(&[
            ascii(Tag::GPSLatitudeRef, "S"),
            degrees(Tag::GPSLatitude, 33, 52, 4, 1),
            ascii(Tag::GPSLongitudeRef, "W"),
            degrees(Tag::GPSLongitude, 70, 39, 0, 1),
        ]);
        let (lat, lon) = position(&exif).unwrap();
        assert!((lat + 33.8678).abs() < 1e-3 && (lon + 70.65).abs() < 1e-3, "{lat} {lon}");
    }

    #[test]
    fn unusable_positions_are_no_position() {
        let at = |lat: Field, lon: Field| position(&exif_of(&[lat, lon]));
        // What cameras without a fix write.
        assert_eq!(at(degrees(Tag::GPSLatitude, 0, 0, 0, 1), degrees(Tag::GPSLongitude, 0, 0, 0, 1)), None);
        // A broken fraction.
        assert_eq!(at(degrees(Tag::GPSLatitude, 40, 25, 1, 0), degrees(Tag::GPSLongitude, 3, 42, 0, 1)), None);
        // Out of range.
        assert_eq!(at(degrees(Tag::GPSLatitude, 95, 0, 0, 1), degrees(Tag::GPSLongitude, 3, 42, 0, 1)), None);
        // Only half of it.
        assert_eq!(position(&exif_of(&[degrees(Tag::GPSLatitude, 40, 25, 1, 1)])), None);
    }
}
