//! Videos: which files are videos, and what their container says about them (size, length,
//! date and place). Nothing is decoded: the browser plays the files as they are.
//!
//! MP4, MOV, M4V and 3GP files (the ISO media format, which phones and most cameras record)
//! are read box by box: the movie header (`mvhd`) gives the length and a UTC creation time,
//! the video track's header (`tkhd`) the size and rotation, and Apple's metadata
//! (`com.apple.quicktime.*`) or `©xyz` the local date and the place. Other formats (WebM,
//! MKV, AVI...) are listed with the file's date and a 16:9 shape.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Local};

const VIDEO_EXTENSIONS: &[&str] =
    &["mp4", "m4v", "mov", "3gp", "webm", "mkv", "avi", "wmv", "mpg", "mpeg", "mts", "m2ts"];
const ISO_MEDIA: &[&str] = &["mp4", "m4v", "mov", "3gp"];

pub fn is_video(path: &Path) -> bool {
    VIDEO_EXTENSIONS.contains(&crate::imaging::extension(path).as_str())
}

/// The media type to serve a video with, for the browser's player.
pub fn mime(path: &Path) -> &'static str {
    match crate::imaging::extension(path).as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "3gp" => "video/3gpp",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        _ => "video/mp2t",
    }
}

/// What a video's container says; each part may be missing.
#[derive(Debug, Default, PartialEq)]
pub struct VideoInfo {
    /// As shown (after the rotation the camera recorded).
    pub width: u32,
    pub height: u32,
    /// Seconds.
    pub duration: Option<f64>,
    /// Local time, "YYYY-MM-DD HH:MM:SS".
    pub taken: Option<String>,
    pub gps: Option<(f64, f64)>,
}

/// Reads what it can; a file it can't make sense of gives an empty `VideoInfo`.
pub fn read(path: &Path) -> Result<VideoInfo> {
    if !ISO_MEDIA.contains(&crate::imaging::extension(path).as_str()) {
        return Ok(VideoInfo::default());
    }
    let mut file = File::open(path)?;
    let Some(moov) = find_moov(&mut file)? else { return Ok(VideoInfo::default()) };
    Ok(parse_moov(&moov))
}

/// The `moov` box's contents. It may come before or after the media data, so the top-level
/// boxes are skipped over (never read) until it is found.
fn find_moov(file: &mut File) -> Result<Option<Vec<u8>>> {
    let len = file.metadata()?.len();
    let mut pos = 0u64;
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos))?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head[..8])?;
        let mut size = u64::from(u32::from_be_bytes(head[..4].try_into()?));
        let kind: [u8; 4] = head[4..8].try_into()?;
        let mut header = 8;
        if size == 1 {
            file.read_exact(&mut head[8..16])?;
            size = u64::from_be_bytes(head[8..16].try_into()?);
            header = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header {
            return Ok(None);
        }
        if &kind == b"moov" {
            // A movie header is small; a huge one is a broken file.
            if size > 64 << 20 {
                return Ok(None);
            }
            let mut moov = vec![0u8; (size - header) as usize];
            file.read_exact(&mut moov)?;
            return Ok(Some(moov));
        }
        pos += size;
    }
    Ok(None)
}

/// The boxes directly inside `data`: (type, contents).
fn boxes(data: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &data[pos + 4..pos + 8];
        let (header, size) = match size {
            0 => (8, data.len() - pos),
            1 if pos + 16 <= data.len() => {
                (16, u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()) as usize)
            }
            n => (8, n),
        };
        if size < header || pos + size > data.len() {
            break;
        }
        out.push((kind, &data[pos + header..pos + size]));
        pos += size;
    }
    out
}

fn child<'a>(data: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
    boxes(data).into_iter().find(|(k, _)| *k == kind).map(|(_, v)| v)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}
fn be64(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

fn parse_moov(moov: &[u8]) -> VideoInfo {
    let mut info = VideoInfo::default();
    let mut utc_created = None;
    if let Some(mvhd) = child(moov, b"mvhd") {
        // version 1 uses 64-bit times and duration
        let (created, timescale, duration) = if mvhd.first() == Some(&1) {
            (be64(mvhd, 4), be32(mvhd, 20), be64(mvhd, 24))
        } else {
            (be32(mvhd, 4).map(u64::from), be32(mvhd, 12), be32(mvhd, 16).map(u64::from))
        };
        if let (Some(scale), Some(d)) = (timescale, duration)
            && scale > 0
        {
            info.duration = Some(d as f64 / f64::from(scale));
        }
        utc_created = created;
    }
    for (kind, trak) in boxes(moov) {
        if kind != b"trak" || !is_video_track(trak) {
            continue;
        }
        if let Some((w, h)) = child(trak, b"tkhd").and_then(track_size) {
            (info.width, info.height) = (w, h);
            break;
        }
    }
    let apple = apple_metadata(moov);
    info.taken = apple
        .iter()
        .find(|(k, _)| k == "com.apple.quicktime.creationdate")
        .and_then(|(_, v)| local_date(v))
        .or_else(|| utc_created.and_then(from_1904));
    info.gps =
        apple.iter().find(|(k, _)| k == "com.apple.quicktime.location.ISO6709").and_then(|(_, v)| iso6709(v)).or_else(
            || {
                child(moov, b"udta").and_then(|u| child(u, b"\xa9xyz")).and_then(|xyz| {
                    // a 16-bit length and a language code, then the text
                    iso6709(&String::from_utf8_lossy(xyz.get(4..)?))
                })
            },
        );
    info
}

fn is_video_track(trak: &[u8]) -> bool {
    child(trak, b"mdia").and_then(|m| child(m, b"hdlr")).and_then(|h| h.get(8..12)) == Some(b"vide")
}

/// Width and height from a track header, turned when the matrix says the video is.
fn track_size(tkhd: &[u8]) -> Option<(u32, u32)> {
    let base = if tkhd.first() == Some(&1) { 4 + 32 } else { 4 + 20 };
    // reserved (8), layer, group, volume, reserved (8), then the 3x3 matrix (36)
    let matrix = base + 16;
    let a = be32(tkhd, matrix)? as i32;
    let b = be32(tkhd, matrix + 4)? as i32;
    let w = be32(tkhd, matrix + 36)? >> 16;
    let h = be32(tkhd, matrix + 40)? >> 16;
    if w == 0 || h == 0 {
        return None;
    }
    // A quarter turn puts the rotation in b, not a.
    Some(if b.unsigned_abs() > a.unsigned_abs() { (h, w) } else { (w, h) })
}

/// QuickTime metadata (`moov/meta`, keys and values), as text.
fn apple_metadata(moov: &[u8]) -> Vec<(String, String)> {
    let Some(meta) = child(moov, b"meta") else { return Vec::new() };
    // QuickTime's meta box has no version/flags; the MP4 one does.
    let meta = if meta.get(4..8) == Some(b"hdlr") { meta } else { meta.get(4..).unwrap_or_default() };
    let Some(keys) = child(meta, b"keys") else { return Vec::new() };
    let mut names = Vec::new();
    let mut pos = 8;
    while let Some(size) = be32(keys, pos) {
        let size = size as usize;
        let Some(name) = keys.get(pos + 8..pos + size) else { break };
        names.push(String::from_utf8_lossy(name).into_owned());
        pos += size.max(8);
    }
    let Some(ilst) = child(meta, b"ilst") else { return Vec::new() };
    boxes(ilst)
        .into_iter()
        .filter_map(|(index, item)| {
            let name = names.get((u32::from_be_bytes(index.try_into().ok()?) as usize).checked_sub(1)?)?;
            let data = child(item, b"data")?;
            Some((name.clone(), String::from_utf8_lossy(data.get(8..)?).into_owned()))
        })
        .collect()
}

/// "2024-10-05T18:22:01+0200" -> "2024-10-05 18:22:01": the time where it was recorded.
fn local_date(text: &str) -> Option<String> {
    let t = text.get(..19)?;
    chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// Seconds since 1904 (UTC) -> this computer's local time. 0 and other unset values are none.
fn from_1904(seconds: u64) -> Option<String> {
    let unix = i64::try_from(seconds).ok()? - 2_082_844_800;
    if unix < 86_400 * 365 {
        return None; // before 1971: not set
    }
    let t = DateTime::from_timestamp(unix, 0)?.with_timezone(&Local);
    Some(t.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// "+40.4168-003.7038+654.000/" -> (40.4168, -3.7038).
fn iso6709(text: &str) -> Option<(f64, f64)> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if (c == '+' || c == '-') && !current.is_empty() {
            numbers.push(current.clone());
            current.clear();
        }
        if c == '/' {
            break;
        }
        current.push(c);
    }
    numbers.push(current);
    let lat: f64 = numbers.first()?.parse().ok()?;
    let lon: f64 = numbers.get(1)?.parse().ok()?;
    let valid = lat.abs() <= 90.0 && lon.abs() <= 180.0 && (lat != 0.0 || lon != 0.0);
    valid.then_some((lat, lon))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    /// A small MOV/MP4 file: a 1920x1080 video track turned a quarter (so shown 1080x1920),
    /// 12.5 s long, created 2024-10-05 16:22:01 UTC, and optionally Apple's local date and
    /// place. The media data is a few bytes, after the movie header or before it.
    pub(crate) fn sample(apple: bool, moov_last: bool) -> Vec<u8> {
        let created: u32 = (1_728_145_321 + 2_082_844_800u64) as u32; // 2024-10-05T16:22:01Z
        let mut mvhd = vec![0u8; 4];
        mvhd.extend(created.to_be_bytes());
        mvhd.extend(created.to_be_bytes());
        mvhd.extend(1000u32.to_be_bytes()); // timescale
        mvhd.extend(12_500u32.to_be_bytes()); // duration
        mvhd.extend([0u8; 80]);
        let mut tkhd = vec![0u8; 4 + 20 + 16];
        // matrix: a quarter turn (a = 0, b = 1, c = -1, d = 0, ...)
        for v in [0i32, 0x10000, 0, -0x10000, 0, 0, 0, 0, 0x4000_0000] {
            tkhd.extend(v.to_be_bytes());
        }
        tkhd.extend((1920u32 << 16).to_be_bytes());
        tkhd.extend((1080u32 << 16).to_be_bytes());
        let mut hdlr = vec![0u8; 8];
        hdlr.extend(b"vide");
        hdlr.extend([0u8; 13]);
        let trak = bx(b"trak", &[bx(b"tkhd", &tkhd), bx(b"mdia", &bx(b"hdlr", &hdlr))].concat());
        let mut moov = [bx(b"mvhd", &mvhd), trak].concat();
        if apple {
            let key = |name: &str| bx(b"mdta", name.as_bytes());
            let mut keys = vec![0u8; 4];
            keys.extend(2u32.to_be_bytes());
            keys.extend(key("com.apple.quicktime.creationdate"));
            keys.extend(key("com.apple.quicktime.location.ISO6709"));
            let value = |index: u32, text: &str| {
                let mut data = 1u32.to_be_bytes().to_vec();
                data.extend([0u8; 4]);
                data.extend(text.as_bytes());
                bx(&index.to_be_bytes(), &bx(b"data", &data))
            };
            let ilst = [value(1, "2024-10-05T18:22:01+0200"), value(2, "+40.4168-003.7038+654.000/")].concat();
            let mut qt_hdlr = vec![0u8; 8];
            qt_hdlr.extend(b"mdta");
            qt_hdlr.extend([0u8; 13]);
            let meta = [bx(b"hdlr", &qt_hdlr), bx(b"keys", &keys), bx(b"ilst", &ilst)].concat();
            moov.extend(bx(b"meta", &meta));
        }
        let (ftyp, moov, mdat) = (bx(b"ftyp", b"qt  \0\0\0\0qt  "), bx(b"moov", &moov), bx(b"mdat", &[7u8; 64]));
        if moov_last { [ftyp, mdat, moov].concat() } else { [ftyp, moov, mdat].concat() }
    }

    fn read_bytes(name: &str, bytes: &[u8]) -> VideoInfo {
        let dir = std::env::temp_dir().join(format!("imadive-video-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let info = read(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        info
    }

    #[test]
    fn an_iphone_video_gives_its_local_date_place_size_and_length() {
        let info = read_bytes("clip.mov", &sample(true, true));
        assert_eq!((info.width, info.height), (1080, 1920), "turned a quarter");
        assert_eq!(info.duration, Some(12.5));
        assert_eq!(info.taken.as_deref(), Some("2024-10-05 18:22:01"), "the local time it was recorded");
        let (lat, lon) = info.gps.unwrap();
        assert!((lat - 40.4168).abs() < 1e-6 && (lon + 3.7038).abs() < 1e-6);
    }

    #[test]
    fn without_apple_metadata_the_utc_creation_time_is_converted_to_local() {
        let info = read_bytes("clip.mp4", &sample(false, false));
        let expected = DateTime::from_timestamp(1_728_145_321, 0).unwrap().with_timezone(&Local);
        assert_eq!(info.taken, Some(expected.format("%Y-%m-%d %H:%M:%S").to_string()));
        assert_eq!(info.gps, None);
        assert_eq!(info.duration, Some(12.5));
    }

    #[test]
    fn other_formats_and_broken_files_give_nothing() {
        assert_eq!(read_bytes("clip.webm", b"\x1aE\xdf\xa3 not read").width, 0);
        assert_eq!(read_bytes("broken.mp4", b"\0\0\0\x05moov"), VideoInfo::default());
        assert_eq!(read_bytes("empty.mov", b""), VideoInfo::default());
    }

    #[test]
    fn iso6709_positions() {
        assert_eq!(iso6709("+40.4168-003.7038/"), Some((40.4168, -3.7038)));
        assert_eq!(iso6709("-33.8678+151.2073+012.000/"), Some((-33.8678, 151.2073)));
        assert_eq!(iso6709("+00.0000+000.0000/"), None, "what cameras write without a fix");
        assert_eq!(iso6709("nonsense"), None);
    }
}
