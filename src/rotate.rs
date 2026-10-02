//! Rotating photos in their files.
//!
//! JPEG photos are turned by changing their EXIF orientation, so the picture itself is not
//! compressed again and loses nothing. PNG is lossless, so it is rewritten turned. Other
//! formats are left alone.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use crate::imaging;

/// Formats that can be turned without harming the file.
pub fn can_rotate(path: &Path) -> bool {
    matches!(imaging::extension(path).as_str(), "jpg" | "jpeg" | "png")
}

pub enum Outcome {
    Rotated {
        width: u32,
        height: u32,
        version: i64,
    },
    NotFound,
    Unsupported,
    /// not inside the photo folders
    Outside,
    /// the file changed since it was indexed: a scan should look at it first
    Changed,
}

/// Turns photo `id` by `turns` quarter turns clockwise, in its file and in the gallery
/// (size, thumbnail and face boxes).
pub fn rotate_photo(conn: &mut Connection, roots: &[PathBuf], id: i64, turns: u8) -> Result<Outcome> {
    let row: Option<(String, i64, i64, i64)> = conn
        .query_row("SELECT path, mtime, size, version FROM photos WHERE id = ?", [id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    let Some((path, mtime, size, version)) = row else { return Ok(Outcome::NotFound) };
    let path = PathBuf::from(path);
    if !roots.iter().any(|r| path.starts_with(r)) {
        return Ok(Outcome::Outside);
    }
    if !can_rotate(&path) {
        return Ok(Outcome::Unsupported);
    }
    let meta = std::fs::metadata(&path)?;
    let indexed_mtime =
        meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    if meta.len() as i64 != size || indexed_mtime != Some(mtime) {
        return Ok(Outcome::Changed);
    }
    let bytes = std::fs::read(&path)?;
    let turns = turns % 4;
    let turned = match turns {
        0 => bytes,
        _ if imaging::extension(&path) == "png" => rotate_png(&path, &bytes, turns)?,
        _ => rotate_jpeg(&bytes, turns)?,
    };
    if turns != 0 {
        replace_file(&path, &turned, &meta)?;
    }

    let img = imaging::decode(&path, &turned)?.into_rgb8();
    let (width, height) = img.dimensions();
    let thumb = crate::scan::thumbnail(&img)?;
    drop(img);
    let meta = std::fs::metadata(&path)?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE photos SET mtime = ?, size = ?, width = ?, height = ?, content_hash = NULL, version = version + 1 WHERE id = ?",
        params![mtime, meta.len() as i64, width, height, id],
    )?;
    tx.execute("UPDATE thumbs SET data = ? WHERE photo_id = ?", params![thumb, id])?;
    // Face boxes turn with the photo. (SQLite reads the old values on the right-hand side.)
    for _ in 0..turns {
        tx.execute("UPDATE faces SET x = 1 - y - h, y = x, w = h, h = w WHERE photo_id = ?", [id])?;
    }
    tx.commit()?;
    Ok(Outcome::Rotated { width, height, version: version + 1 })
}

/// Writes the new contents next to the file and then puts them in its place, so the photo is
/// never left half written. Keeps the file's permissions.
fn replace_file(path: &Path, bytes: &[u8], meta: &std::fs::Metadata) -> Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.totufoto-rotating"));
    let result = (|| -> Result<()> {
        std::fs::write(&tmp, bytes)?;
        std::fs::set_permissions(&tmp, meta.permissions())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn rotate_png(path: &Path, bytes: &[u8], turns: u8) -> Result<Vec<u8>> {
    let img = imaging::decode(path, bytes)?;
    let img = match turns {
        1 => img.rotate90(),
        2 => img.rotate180(),
        _ => img.rotate270(),
    };
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

/// EXIF orientation after `turns` more quarter turns clockwise.
fn turn(orientation: u16, turns: u8) -> u16 {
    const PLAIN: [u16; 4] = [1, 6, 3, 8];
    const MIRRORED: [u16; 4] = [2, 7, 4, 5];
    for cycle in [PLAIN, MIRRORED] {
        if let Some(i) = cycle.iter().position(|&o| o == orientation) {
            return cycle[(i + turns as usize) % 4];
        }
    }
    PLAIN[turns as usize % 4] // an unknown value counts as upright
}

const ORIENTATION: u16 = 0x0112;
const SHORT: u16 = 3;

/// The JPEG with its EXIF orientation turned; the compressed picture is copied untouched.
fn rotate_jpeg(bytes: &[u8], turns: u8) -> Result<Vec<u8>> {
    if bytes.len() < 4 || bytes[..2] != [0xFF, 0xD8] {
        bail!("not a JPEG file");
    }
    let mut pos = 2;
    let mut insert_at = 2;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            bail!("damaged JPEG file");
        }
        let marker = bytes[pos + 1];
        match marker {
            0xFF => {
                pos += 1; // fill byte
                continue;
            }
            0xDA | 0xD9 => break, // the picture starts: no EXIF before it
            0x01 | 0xD0..=0xD7 => {
                pos += 2;
                continue;
            }
            _ => {}
        }
        let len = u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        if len < 2 || pos + 2 + len > bytes.len() {
            bail!("damaged JPEG file");
        }
        let data = &bytes[pos + 4..pos + 2 + len];
        if marker == 0xE1 && data.starts_with(b"Exif\0\0") {
            let tiff = set_orientation(&data[6..], turns)?;
            return splice_exif(bytes, pos, pos + 2 + len, &tiff);
        }
        if marker == 0xE0 && insert_at == pos {
            insert_at = pos + 2 + len; // after the JFIF header at the start
        }
        pos += 2 + len;
    }
    // No EXIF yet: a minimal one with only the orientation.
    let mut tiff = b"MM\0\x2a\0\0\0\x08\0\x01".to_vec();
    tiff.extend_from_slice(&ORIENTATION.to_be_bytes());
    tiff.extend_from_slice(&SHORT.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&turn(1, turns).to_be_bytes());
    tiff.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // value padding, then no next IFD
    splice_exif(bytes, insert_at, insert_at, &tiff)
}

/// `bytes` with `start..end` replaced by an EXIF segment holding `tiff`.
fn splice_exif(bytes: &[u8], start: usize, end: usize, tiff: &[u8]) -> Result<Vec<u8>> {
    let len = tiff.len() + 8;
    if len > 0xFFFF {
        bail!("its EXIF data is too large to change");
    }
    let mut out = Vec::with_capacity(bytes.len() + 64);
    out.extend_from_slice(&bytes[..start]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(&bytes[end..]);
    Ok(out)
}

/// EXIF (TIFF) data with the orientation in its first IFD turned. When it has none, a copy
/// of that IFD with the tag added goes at the end; everything else stays where it was, so
/// the offsets in it remain right.
fn set_orientation(tiff: &[u8], turns: u8) -> Result<Vec<u8>> {
    let be = match tiff.get(..2) {
        Some(b"MM") => true,
        Some(b"II") => false,
        _ => bail!("damaged EXIF data"),
    };
    let damaged = || anyhow::anyhow!("damaged EXIF data");
    let r16 = |o: usize| -> Result<u16> {
        let b = tiff.get(o..o + 2).ok_or_else(damaged)?;
        Ok(if be { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    };
    let r32 = |o: usize| -> Result<u32> {
        let b: [u8; 4] = tiff.get(o..o + 4).ok_or_else(damaged)?.try_into()?;
        Ok(if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    };
    let b16 = |v: u16| if be { v.to_be_bytes() } else { v.to_le_bytes() };
    let b32 = |v: u32| if be { v.to_be_bytes() } else { v.to_le_bytes() };
    if r16(2)? != 42 {
        bail!("damaged EXIF data");
    }
    let ifd = r32(4)? as usize;
    let count = r16(ifd)? as usize;
    let entries = ifd + 2;
    if entries + 12 * count + 4 > tiff.len() {
        bail!("damaged EXIF data");
    }
    let mut out = tiff.to_vec();
    for i in 0..count {
        let e = entries + 12 * i;
        if r16(e)? == ORIENTATION {
            let current = if r16(e + 2)? == SHORT { r16(e + 8)? } else { 1 };
            out[e + 2..e + 4].copy_from_slice(&b16(SHORT));
            out[e + 4..e + 8].copy_from_slice(&b32(1));
            out[e + 8..e + 10].copy_from_slice(&b16(turn(current, turns)));
            out[e + 10..e + 12].fill(0);
            return Ok(out);
        }
    }
    let mut list: Vec<[u8; 12]> =
        (0..count).map(|i| tiff[entries + 12 * i..entries + 12 * i + 12].try_into().unwrap()).collect();
    let mut added = [0u8; 12];
    added[..2].copy_from_slice(&b16(ORIENTATION));
    added[2..4].copy_from_slice(&b16(SHORT));
    added[4..8].copy_from_slice(&b32(1));
    added[8..10].copy_from_slice(&b16(turn(1, turns)));
    let tag = |e: &[u8; 12]| if be { u16::from_be_bytes([e[0], e[1]]) } else { u16::from_le_bytes([e[0], e[1]]) };
    let at = list.iter().position(|e| tag(e) > ORIENTATION).unwrap_or(list.len());
    list.insert(at, added);
    if out.len() % 2 == 1 {
        out.push(0); // IFDs start on a word boundary
    }
    let new_ifd = out.len();
    out.extend_from_slice(&b16(list.len() as u16));
    for e in &list {
        out.extend_from_slice(e);
    }
    out.extend_from_slice(&tiff[entries + 12 * count..entries + 12 * count + 4]); // next IFD
    out[4..8].copy_from_slice(&b32(new_ifd as u32));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// 40×20, white with a red block in the top left corner.
    fn sample() -> RgbImage {
        let mut img = RgbImage::from_pixel(40, 20, Rgb([255, 255, 255]));
        for y in 0..6 {
            for x in 0..8 {
                img.put_pixel(x, y, Rgb([255, 0, 0]));
            }
        }
        img
    }

    fn red(img: &RgbImage, x: u32, y: u32) -> bool {
        let p = img.get_pixel(x, y);
        p[0] > 200 && p[1] < 80 && p[2] < 80
    }

    /// Where the red block shows: (right side, bottom side).
    fn corner(bytes: &[u8]) -> (u32, u32, bool, bool) {
        let img = imaging::decode(Path::new("x.jpg"), bytes).unwrap().into_rgb8();
        let (w, h) = img.dimensions();
        let right = red(&img, w - 2, 2) || red(&img, w - 2, h - 2);
        let bottom = red(&img, 2, h - 2) || red(&img, w - 2, h - 2);
        assert!(red(&img, 2, 2) || red(&img, w - 2, 2) || red(&img, 2, h - 2) || red(&img, w - 2, h - 2));
        (w, h, right, bottom)
    }

    fn exif_field(bytes: &[u8], tag: exif::Tag) -> Option<String> {
        let exif = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)).ok()?;
        exif.get_field(tag, exif::In::PRIMARY).map(|f| f.display_value().to_string())
    }

    #[test]
    fn orientation_cycles() {
        assert_eq!(turn(1, 1), 6);
        assert_eq!(turn(6, 1), 3);
        assert_eq!(turn(8, 1), 1);
        assert_eq!(turn(1, 3), 8);
        assert_eq!(turn(2, 1), 7);
        assert_eq!(turn(5, 1), 2);
        assert_eq!(turn(0, 2), 3);
    }

    #[test]
    fn jpeg_without_exif() {
        let original = imaging::encode_jpeg(&sample(), 95).unwrap();
        assert_eq!(corner(&original), (40, 20, false, false));
        let once = rotate_jpeg(&original, 1).unwrap();
        assert_eq!(corner(&once), (20, 40, true, false), "clockwise: top left goes to top right");
        let twice = rotate_jpeg(&once, 1).unwrap();
        assert_eq!(corner(&twice), (40, 20, true, true));
        assert_eq!(twice.len(), once.len(), "an existing orientation is changed in place");
        let back = rotate_jpeg(&twice, 2).unwrap();
        assert_eq!(corner(&back), (40, 20, false, false));
        let left = rotate_jpeg(&original, 3).unwrap();
        assert_eq!(corner(&left), (20, 40, false, true));
        // the picture data is untouched
        let scan = |b: &[u8]| b.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        assert_eq!(&original[scan(&original)..], &once[scan(&once)..]);
    }

    /// An EXIF block with a camera name but no orientation (both byte orders).
    fn with_make(big_endian: bool) -> Vec<u8> {
        let jpeg = imaging::encode_jpeg(&sample(), 95).unwrap();
        let b16 = |v: u16| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let b32 = |v: u32| if big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut tiff = if big_endian { b"MM".to_vec() } else { b"II".to_vec() };
        tiff.extend_from_slice(&b16(42));
        tiff.extend_from_slice(&b32(8));
        tiff.extend_from_slice(&b16(1));
        tiff.extend_from_slice(&b16(0x010F)); // Make
        tiff.extend_from_slice(&b16(2)); // ASCII
        tiff.extend_from_slice(&b32(9));
        tiff.extend_from_slice(&b32(26)); // offset of the text
        tiff.extend_from_slice(&b32(0));
        tiff.extend_from_slice(b"TotuCam1\0");
        splice_exif(&jpeg, 2, 2, &tiff).unwrap()
    }

    #[test]
    fn jpeg_with_exif_but_no_orientation() {
        for big_endian in [true, false] {
            let original = with_make(big_endian);
            assert_eq!(exif_field(&original, exif::Tag::Make).as_deref(), Some("\"TotuCam1\""));
            let once = rotate_jpeg(&original, 1).unwrap();
            assert_eq!(corner(&once), (20, 40, true, false));
            assert_eq!(exif_field(&once, exif::Tag::Make).as_deref(), Some("\"TotuCam1\""), "other EXIF data kept");
            assert_eq!(
                exif_field(&once, exif::Tag::Orientation).as_deref(),
                Some("row 0 at right and column 0 at top")
            );
            let twice = rotate_jpeg(&once, 1).unwrap();
            assert_eq!(corner(&twice), (40, 20, true, true));
        }
    }

    #[test]
    fn png_is_turned() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(sample()).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let turned = rotate_png(Path::new("x.png"), png.get_ref(), 1).unwrap();
        let img = imaging::decode(Path::new("x.png"), &turned).unwrap().into_rgb8();
        assert_eq!(img.dimensions(), (20, 40));
        assert!(red(&img, 18, 2), "top left goes to top right");
    }

    #[test]
    fn not_a_jpeg() {
        assert!(rotate_jpeg(b"hello", 1).is_err());
    }

    use crate::testutil::{Library, Photo};

    fn face_box(conn: &Connection) -> (f64, f64, f64, f64) {
        conn.query_row("SELECT x, y, w, h FROM faces", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
    }

    #[test]
    fn rotating_updates_the_gallery_too() {
        let lib = Library::new();
        lib.add("a.jpg", Photo { width: 64, height: 48, ..Photo::default() });
        lib.scan();
        let id = lib.id("a.jpg");
        let mut conn = lib.conn();
        conn.execute(
            "INSERT INTO faces (photo_id, x, y, w, h, score, embedding, thumb) VALUES (?, 0.1, 0.2, 0.3, 0.4, 1, x'', x'')",
            [id],
        )
        .unwrap();

        let Outcome::Rotated { width, height, version } = rotate_photo(&mut conn, &[lib.root()], id, 1).unwrap() else {
            panic!("not rotated")
        };
        assert_eq!((width, height, version), (48, 64, 1));
        let (x, y, w, h) = face_box(&conn);
        // A quarter turn clockwise: the left edge becomes the top one.
        for (got, want) in [(x, 0.4), (y, 0.1), (w, 0.4), (h, 0.3)] {
            assert!((got - want).abs() < 1e-9, "{:?}", face_box(&conn));
        }
        let img = crate::imaging::decode(&lib.root().join("a.jpg"), &std::fs::read(lib.root().join("a.jpg")).unwrap())
            .unwrap();
        assert_eq!((img.width(), img.height()), (48, 64), "the file itself is turned");

        // The next scan sees an unchanged file: same photo, same face.
        let status = lib.scan();
        assert_eq!(status.total, 0);
        assert_eq!(lib.id("a.jpg"), id);
        assert!((face_box(&lib.conn()).0 - 0.4).abs() < 1e-9);

        // Three more quarter turns bring the box back.
        rotate_photo(&mut conn, &[lib.root()], id, 3).unwrap();
        let (x, y, w, h) = face_box(&conn);
        for (got, want) in [(x, 0.1), (y, 0.2), (w, 0.3), (h, 0.4)] {
            assert!((got - want).abs() < 1e-9, "{:?}", face_box(&conn));
        }
    }

    #[test]
    fn rotating_is_refused_when_it_would_be_unsafe() {
        let lib = Library::new();
        lib.add("a.jpg", Photo::default());
        std::fs::write(lib.dir.file("photos/b.gif"), b"GIF89a").unwrap();
        lib.scan();
        let mut conn = lib.conn();
        let id = lib.id("a.jpg");
        let elsewhere = lib.dir.path().join("elsewhere");
        assert!(matches!(rotate_photo(&mut conn, &[elsewhere], id, 1).unwrap(), Outcome::Outside));
        assert!(matches!(rotate_photo(&mut conn, &[lib.root()], 9999, 1).unwrap(), Outcome::NotFound));
        // Changed since it was indexed: a scan should look at it first.
        lib.add("a.jpg", Photo { width: 10, height: 10, ..Photo::default() });
        assert!(matches!(rotate_photo(&mut conn, &[lib.root()], id, 1).unwrap(), Outcome::Changed));
        // A GIF in the index (inserted directly; the broken test file doesn't index).
        conn.execute(
            "INSERT INTO photos (path, mtime, size, width, height, taken, date_from_exif) VALUES (?, 0, 6, 1, 1, '2020-01-01 00:00:00', 0)",
            [lib.root().join("b.gif").to_string_lossy()],
        )
        .unwrap();
        let gif = conn.last_insert_rowid();
        assert!(matches!(rotate_photo(&mut conn, &[lib.root()], gif, 1).unwrap(), Outcome::Unsupported));
    }
}
