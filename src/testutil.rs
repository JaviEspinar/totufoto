//! Helpers for tests: temporary folders and generated photos with EXIF data.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use image::{Rgb, RgbImage};

/// `rel` ("a/b.jpg") as a path with this system's separator, so it compares equal to the
/// paths the scan stores (on Windows, `dir.join("a/b.jpg")` keeps the forward slash).
pub fn native(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

/// A folder under the system's temporary folder, deleted (with its contents) when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("totufoto-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temporary folder");
        // Canonical, like the folders the app stores (on macOS /tmp is a link to /private/tmp).
        Self(dunce::canonicalize(&path).expect("temporary folder"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `rel` inside this folder, with its parent folders created.
    pub fn file(&self, rel: &str) -> PathBuf {
        let path = self.0.join(native(rel));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A photo library in a temporary folder: photos under `photos/` (given as a command-line
/// folder) and the index under `data/`, scanned without face recognition.
pub struct Library {
    pub dir: TempDir,
    pub cfg: crate::scan::ScanConfig,
}

impl Library {
    pub fn new() -> Self {
        let dir = TempDir::new();
        let photos = dir.file("photos/.keep").parent().unwrap().to_path_buf();
        let cfg = crate::scan::ScanConfig {
            fixed_roots: vec![photos],
            db_path: dir.file("data/index.sqlite"),
            models: None,
            cluster_threshold: 0.42,
        };
        Self { dir, cfg }
    }

    /// The photos folder.
    pub fn root(&self) -> PathBuf {
        self.cfg.fixed_roots[0].clone()
    }

    /// Writes `photo` at `rel` inside the photos folder and returns its path.
    pub fn add(&self, rel: &str, photo: Photo) -> PathBuf {
        let path = self.dir.file(&format!("photos/{rel}"));
        photo.write(&path);
        path
    }

    pub fn scan(&self) -> crate::scan::StatusView {
        let status = crate::scan::ScanStatus::default();
        crate::scan::run(&self.cfg, &status).expect("scan");
        status.view()
    }

    pub fn conn(&self) -> rusqlite::Connection {
        crate::db::open(&self.cfg.db_path).expect("index")
    }

    /// The id of the photo at `rel` inside the photos folder.
    pub fn id(&self, rel: &str) -> i64 {
        let path = self.root().join(native(rel));
        self.conn()
            .query_row("SELECT id FROM photos WHERE path = ?", [path.to_string_lossy()], |r| r.get(0))
            .expect(rel)
    }
}

/// Sets a file's modification time, `secs` after 2000-01-01.
pub fn set_mtime(path: &Path, secs: u64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(946_684_800 + secs);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
}

/// What to put in a generated photo.
#[derive(Clone, Copy)]
pub struct Photo {
    pub width: u32,
    pub height: u32,
    /// The whole picture is this colour; photos with different colours are different files.
    pub color: [u8; 3],
    /// EXIF DateTimeOriginal, as "YYYY:MM:DD HH:MM:SS".
    pub taken: Option<&'static str>,
    /// EXIF GPS position (latitude, longitude) in degrees.
    pub gps: Option<(f64, f64)>,
}

impl Default for Photo {
    fn default() -> Self {
        Self { width: 64, height: 48, color: [200, 120, 40], taken: None, gps: None }
    }
}

impl Photo {
    /// The JPEG file's bytes.
    pub fn jpeg(&self) -> Vec<u8> {
        let img = RgbImage::from_pixel(self.width, self.height, Rgb(self.color));
        let jpeg = crate::imaging::encode_jpeg(&img, 90).expect("JPEG");
        match self.exif() {
            Some(tiff) => with_exif(&jpeg, &tiff),
            None => jpeg,
        }
    }

    /// Writes the photo to `path`.
    pub fn write(&self, path: &Path) {
        std::fs::write(path, self.jpeg()).expect("writing a test photo");
    }

    fn exif(&self) -> Option<Vec<u8>> {
        use exif::{Field, In, Rational, Tag, Value};
        if self.taken.is_none() && self.gps.is_none() {
            return None;
        }
        let mut fields = Vec::new();
        if let Some(taken) = self.taken {
            fields.push(Field {
                tag: Tag::DateTimeOriginal,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![taken.into()]),
            });
        }
        if let Some((lat, lon)) = self.gps {
            let dms = |v: f64| {
                let v = v.abs();
                let deg = v.floor();
                let min = ((v - deg) * 60.0).floor();
                let sec = ((v - deg) * 60.0 - min) * 60.0;
                Value::Rational(vec![
                    Rational { num: deg as u32, denom: 1 },
                    Rational { num: min as u32, denom: 1 },
                    Rational { num: (sec * 1000.0).round() as u32, denom: 1000 },
                ])
            };
            let ns = if lat < 0.0 { "S" } else { "N" };
            let ew = if lon < 0.0 { "W" } else { "E" };
            fields.push(Field { tag: Tag::GPSLatitudeRef, ifd_num: In::PRIMARY, value: Value::Ascii(vec![ns.into()]) });
            fields.push(Field { tag: Tag::GPSLatitude, ifd_num: In::PRIMARY, value: dms(lat) });
            fields.push(Field {
                tag: Tag::GPSLongitudeRef,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![ew.into()]),
            });
            fields.push(Field { tag: Tag::GPSLongitude, ifd_num: In::PRIMARY, value: dms(lon) });
        }
        let mut writer = exif::experimental::Writer::new();
        for f in &fields {
            writer.push_field(f);
        }
        let mut tiff = Cursor::new(Vec::new());
        writer.write(&mut tiff, false).expect("EXIF");
        Some(tiff.into_inner())
    }
}

/// The JPEG with an EXIF segment holding `tiff` right after its start marker.
fn with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(jpeg.len() + tiff.len() + 10);
    out.extend_from_slice(&jpeg[..2]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(&jpeg[2..]);
    out
}

#[test]
fn generated_photos_carry_their_exif() {
    let bytes = Photo { taken: Some("2021:06:15 10:30:00"), gps: Some((40.4168, -3.7038)), ..Photo::default() }.jpeg();
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(&bytes)).expect("EXIF");
    let taken = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY).expect("date");
    assert_eq!(taken.display_value().to_string(), "2021-06-15 10:30:00");
    let lat = exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY).expect("latitude");
    assert!(lat.display_value().to_string().starts_with("40 deg 25 min"), "{}", lat.display_value());
    let img = crate::imaging::decode(Path::new("x.jpg"), &bytes).expect("decodes").into_rgb8();
    assert_eq!(img.dimensions(), (64, 48));
}
