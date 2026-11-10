//! Pure pieces of the freedesktop Thumbnail Managing Standard: canonical
//! `file://` URIs, MD5 cache names, a bounded PNG metadata reader for
//! untrusted cache files, and a PNG writer for thumbnails Files generated.
//!
//! No pixel data is decompressed here. Cache PNG pixels are decoded by the
//! ephemeral helper process; this module only walks chunk headers and reads
//! small uncompressed text chunks, so a hostile cache file cannot make the
//! UI process inflate or allocate more than [`MAX_TEXT_CHUNK`] bytes.
use std::{
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path},
};

/// Largest cache PNG accepted from the shared directory.
pub const MAX_CACHE_FILE: u64 = 512 * 1024;
/// Largest tEXt/iTXt chunk read into memory; bigger ones are skipped unread.
pub const MAX_TEXT_CHUNK: u32 = 16 * 1024;
/// Chunk walk bound: real thumbnails have fewer than a dozen chunks.
pub const MAX_CHUNKS: usize = 1024;
/// `normal/` thumbnails written by Files are at most this many pixels.
pub const NORMAL_SIZE: u32 = 128;
/// `large/` thumbnails read by Files are at most this many pixels.
pub const LARGE_SIZE: u32 = 256;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Canonical `file://` URI for an absolute local path, escaped exactly like
/// GLib's `g_filename_to_uri` (the reference used by GNOME thumbnailers):
/// ASCII alphanumerics and `!$&'()*+,-./:=@_~` stay literal, every other
/// byte (including non-UTF-8 and non-ASCII bytes) becomes `%XX` uppercase.
/// Relative paths and paths containing `..` have no canonical URI here.
pub fn file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let mut uri = String::from("file://");
    let mut any = false;
    for part in path.components() {
        match part {
            Component::RootDir => {}
            Component::Normal(name) => {
                any = true;
                uri.push('/');
                for &b in name.as_encoded_bytes() {
                    if b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b) {
                        uri.push(b as char);
                    } else {
                        const HEX: &[u8; 16] = b"0123456789ABCDEF";
                        uri.push('%');
                        uri.push(HEX[usize::from(b >> 4)] as char);
                        uri.push(HEX[usize::from(b & 15)] as char);
                    }
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    if !any {
        uri.push('/');
    }
    Some(uri)
}

/// `MD5(uri)` as 32 lowercase hex digits followed by `.png`.
pub fn cache_name(uri: &str) -> String {
    let mut name = String::with_capacity(36);
    for b in md5(uri.as_bytes()) {
        name.push_str(&format!("{b:02x}"));
    }
    name.push_str(".png");
    name
}

/// RFC 1321 MD5. Used only as the standard's cache file name, never as a
/// security or integrity primitive.
pub fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] =
        std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32);
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((input.len() as u64).wrapping_mul(8)).to_le_bytes());
    for block in message.as_chunks::<64>().0 {
        let m: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(k[i])
                .wrapping_add(m[g])
                .rotate_left(S[i]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(rotated);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut out = [0u8; 16];
    for (i, s) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
    }
    out
}

/// Metadata of a cache PNG. Pixels are not inspected.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub width: u32,
    pub height: u32,
    pub uri: Option<Vec<u8>>,
    pub mtime: Option<i64>,
    pub size: Option<u64>,
}

impl Meta {
    /// The standard's validity rule: same URI, same mtime and, when the
    /// optional `Thumb::Size` is present, the same byte size.
    pub fn matches(&self, uri: &str, mtime: i64, size: u64) -> bool {
        self.uri.as_deref() == Some(uri.as_bytes())
            && self.mtime == Some(mtime)
            && self.size.is_none_or(|s| s == size)
    }
}

fn decimal_i64(value: &[u8]) -> Option<i64> {
    let digits = value.strip_prefix(b"-").unwrap_or(value);
    if digits.is_empty() || digits.len() > 19 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(value).ok()?.parse().ok()
}

/// Walk a cache PNG's chunks within `file_len` bytes. Requires the PNG
/// signature, a first 13-byte IHDR whose dimensions are 1..=`max_side`, and
/// a reachable IEND. Text chunks over [`MAX_TEXT_CHUNK`] are skipped; a
/// duplicated `Thumb::` key or a bad text-chunk CRC rejects the file.
pub fn read_meta<R: Read + Seek>(r: &mut R, file_len: u64, max_side: u32) -> io::Result<Meta> {
    if file_len > MAX_CACHE_FILE {
        return Err(invalid("Miniatura compartida demasiado grande"));
    }
    let mut head = [0u8; 8];
    r.read_exact(&mut head)?;
    if head != SIGNATURE {
        return Err(invalid("La miniatura compartida no es PNG"));
    }
    let mut at = 8u64;
    let mut meta = Meta::default();
    for index in 0..MAX_CHUNKS {
        let mut header = [0u8; 8];
        r.read_exact(&mut header)?;
        let len = u32::from_be_bytes(header[..4].try_into().unwrap());
        let kind: [u8; 4] = header[4..].try_into().unwrap();
        let end = at
            .checked_add(12)
            .and_then(|n| n.checked_add(u64::from(len)))
            .filter(|n| *n <= file_len)
            .ok_or_else(|| invalid("Fragmento PNG fuera del archivo"))?;
        if index == 0 {
            if &kind != b"IHDR" || len != 13 {
                return Err(invalid("PNG sin IHDR inicial"));
            }
            let mut ihdr = [0u8; 17];
            r.read_exact(&mut ihdr)?;
            meta.width = u32::from_be_bytes(ihdr[..4].try_into().unwrap());
            meta.height = u32::from_be_bytes(ihdr[4..8].try_into().unwrap());
            if !(1..=max_side).contains(&meta.width) || !(1..=max_side).contains(&meta.height) {
                return Err(invalid("Dimensiones de miniatura compartida no válidas"));
            }
        } else if &kind == b"IHDR" {
            return Err(invalid("IHDR duplicado"));
        } else if &kind == b"IEND" {
            return Ok(meta);
        } else if (&kind == b"tEXt" || &kind == b"iTXt") && len <= MAX_TEXT_CHUNK {
            let mut data = vec![0u8; len as usize + 4];
            r.read_exact(&mut data)?;
            let (body, crc) = data.split_at(len as usize);
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(&kind);
            hasher.update(body);
            if hasher.finalize() != u32::from_be_bytes(crc.try_into().unwrap()) {
                return Err(invalid("CRC de texto PNG no válido"));
            }
            if let Some((key, value)) = text_entry(&kind, body) {
                let slot_taken = match key {
                    b"Thumb::URI" => meta.uri.replace(value.to_vec()).is_some(),
                    b"Thumb::MTime" => meta
                        .mtime
                        .replace(decimal_i64(value).ok_or_else(|| invalid("Thumb::MTime"))?)
                        .is_some(),
                    b"Thumb::Size" => meta
                        .size
                        .replace(
                            decimal_i64(value)
                                .and_then(|n| u64::try_from(n).ok())
                                .ok_or_else(|| invalid("Thumb::Size"))?,
                        )
                        .is_some(),
                    _ => false,
                };
                if slot_taken {
                    return Err(invalid("Clave Thumb duplicada"));
                }
            }
        } else {
            r.seek(SeekFrom::Start(end))?;
        }
        at = end;
    }
    Err(invalid("Demasiados fragmentos PNG"))
}

/// Keyword and value of an uncompressed tEXt or iTXt chunk. Compressed iTXt
/// and zTXt are ignored: this module never inflates untrusted data.
fn text_entry<'a>(kind: &[u8; 4], body: &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
    let split = body.iter().position(|b| *b == 0)?;
    let (key, rest) = (&body[..split], &body[split + 1..]);
    if kind == b"tEXt" {
        return Some((key, rest));
    }
    // iTXt: compression flag, method, language\0, translated keyword\0, text.
    let (&flag, rest) = rest.split_first()?;
    let (_, rest) = rest.split_first()?;
    if flag != 0 {
        return None;
    }
    let lang = rest.iter().position(|b| *b == 0)?;
    let rest = &rest[lang + 1..];
    let translated = rest.iter().position(|b| *b == 0)?;
    Some((key, &rest[translated + 1..]))
}

/// Box-filter an RGB image so its longest side is at most `max_side`.
/// Images already within the bound are returned unchanged (no upscaling).
pub fn fit_rgb(width: u32, height: u32, rgb: &[u8], max_side: u32) -> (u32, u32, Vec<u8>) {
    if width <= max_side && height <= max_side {
        return (width, height, rgb.to_vec());
    }
    let longest = width.max(height);
    let w = (u64::from(width) * u64::from(max_side) / u64::from(longest)).max(1) as u32;
    let h = (u64::from(height) * u64::from(max_side) / u64::from(longest)).max(1) as u32;
    let mut out = Vec::with_capacity(w as usize * h as usize * 3);
    for y in 0..h {
        let y0 = y * height / h;
        let y1 = ((y + 1) * height / h).max(y0 + 1);
        for x in 0..w {
            let x0 = x * width / w;
            let x1 = ((x + 1) * width / w).max(x0 + 1);
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy as usize * width as usize + sx as usize) * 3;
                    for c in 0..3 {
                        sum[c] += u32::from(rgb[i + c]);
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            out.extend(sum.map(|s| ((s + n / 2) / n) as u8));
        }
    }
    (w, h, out)
}

/// Encode an 8-bit RGB thumbnail as PNG with the standard's tEXt keys.
pub fn encode_png(
    width: u32,
    height: u32,
    rgb: &[u8],
    uri: &str,
    mtime: i64,
    size: u64,
) -> io::Result<Vec<u8>> {
    if width == 0
        || height == 0
        || width > LARGE_SIZE
        || height > LARGE_SIZE
        || rgb.len() != width as usize * height as usize * 3
    {
        return Err(invalid("Miniatura para caché compartida fuera de límite"));
    }
    if uri.len() as u32 > MAX_TEXT_CHUNK - 64 {
        return Err(invalid("URI demasiado larga para la caché compartida"));
    }
    let mut png = Vec::with_capacity(rgb.len() / 2 + 1024);
    png.extend_from_slice(&SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    for (key, value) in [
        ("Thumb::URI", uri.to_owned()),
        ("Thumb::MTime", mtime.to_string()),
        ("Thumb::Size", size.to_string()),
        ("Software", "Files".to_owned()),
    ] {
        let mut text = Vec::with_capacity(key.len() + 1 + value.len());
        text.extend_from_slice(key.as_bytes());
        text.push(0);
        text.extend_from_slice(value.as_bytes());
        chunk(&mut png, b"tEXt", &text);
    }
    let mut z = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(rgb.len() / 2),
        flate2::Compression::default(),
    );
    for row in rgb.chunks_exact(width as usize * 3) {
        z.write_all(&[0])?;
        z.write_all(row)?;
    }
    chunk(&mut png, b"IDAT", &z.finish()?);
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn hex(bytes: [u8; 16]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn md5_matches_rfc_1321_vectors() {
        assert_eq!(hex(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        // Crosses the 56-byte padding boundary into a second block.
        assert_eq!(hex(md5(&[b'a'; 64])), "014842d480b571495a4a0363793f7367");
    }

    #[test]
    fn uri_and_cache_name_follow_the_standard_and_glib_escaping() {
        // The standard's own example.
        let uri = file_uri(Path::new("/home/jens/photos/me.png")).unwrap();
        assert_eq!(uri, "file:///home/jens/photos/me.png");
        assert_eq!(cache_name(&uri), "c6ee772d9e49320e97ec29a7eb5b1697.png");
        // Space, non-ASCII UTF-8, '#' and a literal '%' are percent-encoded.
        let uri = file_uri(Path::new("/tmp/a b/café#%.png")).unwrap();
        assert_eq!(uri, "file:///tmp/a%20b/caf%C3%A9%23%25.png");
        assert_eq!(cache_name(&uri), "b8b252f9e1324caf3d61c5e87befdf46.png");
        // GLib's path-safe set stays literal; brackets, '?', ';' and braces do not.
        let uri = file_uri(Path::new("/tmp/[x]?;{}/!$&'()*+,-.:=@_~.png")).unwrap();
        assert_eq!(uri, "file:///tmp/%5Bx%5D%3F%3B%7B%7D/!$&'()*+,-.:=@_~.png");
        assert_eq!(cache_name(&uri), "e34f009e09fa35644b3bb40da1cc8307.png");
        // Non-UTF-8 bytes travel losslessly as escapes; dot segments normalize.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let raw = Path::new(std::ffi::OsStr::from_bytes(b"/x/./\xff\xfe"));
            assert_eq!(file_uri(raw).unwrap(), "file:///x/%FF%FE");
        }
        assert_eq!(file_uri(Path::new("/")).unwrap(), "file:///");
        assert!(file_uri(Path::new("relative.png")).is_none());
        assert!(file_uri(Path::new("/a/../b.png")).is_none());
    }

    #[test]
    fn written_png_carries_standard_keys_and_reads_back() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|n| n as u8).collect();
        let png = encode_png(4, 3, &rgb, "file:///a%20b.png", 1_700_000_000, 42).unwrap();
        let meta = read_meta(&mut Cursor::new(&png), png.len() as u64, NORMAL_SIZE).unwrap();
        assert_eq!((meta.width, meta.height), (4, 3));
        assert_eq!(meta.uri.as_deref(), Some(&b"file:///a%20b.png"[..]));
        assert_eq!(meta.mtime, Some(1_700_000_000));
        assert_eq!(meta.size, Some(42));
        assert!(meta.matches("file:///a%20b.png", 1_700_000_000, 42));
        assert!(!meta.matches("file:///a%20b.png", 1_700_000_001, 42));
        assert!(!meta.matches("file:///a b.png", 1_700_000_000, 42));
        assert!(!meta.matches("file:///a%20b.png", 1_700_000_000, 43));
        // The IDAT holds exactly the filtered rows.
        let start = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[start - 4..start].try_into().unwrap()) as usize;
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&png[start + 4..start + 4 + len])
            .read_to_end(&mut raw)
            .unwrap();
        assert_eq!(raw.len(), 3 * (1 + 12));
        assert_eq!(&raw[1..13], &rgb[..12]);
        assert!(encode_png(0, 3, &[], "file:///x", 0, 0).is_err());
        assert!(encode_png(257, 1, &vec![0; 257 * 3], "file:///x", 0, 0).is_err());
    }

    #[test]
    fn size_key_is_optional_but_must_match_when_present() {
        let meta = Meta {
            width: 1,
            height: 1,
            uri: Some(b"file:///x".to_vec()),
            mtime: Some(5),
            size: None,
        };
        assert!(meta.matches("file:///x", 5, 999));
        let missing_mtime = Meta {
            mtime: None,
            ..meta
        };
        assert!(!missing_mtime.matches("file:///x", 5, 999));
    }

    fn png_with(extra: &[(&[u8; 4], Vec<u8>)], width: u32) -> Vec<u8> {
        let mut png = SIGNATURE.to_vec();
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&1u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(&mut png, b"IHDR", &ihdr);
        for (kind, data) in extra {
            chunk(&mut png, kind, data);
        }
        chunk(&mut png, b"IDAT", &[0; 8]);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn malformed_oversized_or_ambiguous_cache_files_are_rejected() {
        let ok = png_with(&[(b"tEXt", b"Thumb::URI\0file:///x".to_vec())], 128);
        let len = ok.len() as u64;
        assert!(read_meta(&mut Cursor::new(&ok), len, NORMAL_SIZE).is_ok());
        // Dimensions over the directory's size or zero.
        let wide = png_with(&[], 129);
        assert!(read_meta(&mut Cursor::new(&wide), wide.len() as u64, NORMAL_SIZE).is_err());
        let zero = png_with(&[], 0);
        assert!(read_meta(&mut Cursor::new(&zero), zero.len() as u64, LARGE_SIZE).is_err());
        // Absurd file size is refused before reading anything.
        assert!(read_meta(&mut Cursor::new(&ok), MAX_CACHE_FILE + 1, NORMAL_SIZE).is_err());
        // Truncated: no IEND within the file, or a chunk running past the end.
        let cut = &ok[..ok.len() - 12];
        assert!(read_meta(&mut Cursor::new(cut), cut.len() as u64, NORMAL_SIZE).is_err());
        let mut lying = ok.clone();
        let idat = lying.windows(4).position(|w| w == b"IDAT").unwrap();
        lying[idat - 4..idat].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(read_meta(&mut Cursor::new(&lying), lying.len() as u64, NORMAL_SIZE).is_err());
        // Not a PNG.
        assert!(read_meta(&mut Cursor::new(b"GIF89a.........."), 16, NORMAL_SIZE).is_err());
        // Duplicate URI keys are ambiguous.
        let dup = png_with(
            &[
                (b"tEXt", b"Thumb::URI\0file:///x".to_vec()),
                (b"tEXt", b"Thumb::URI\0file:///y".to_vec()),
            ],
            8,
        );
        assert!(read_meta(&mut Cursor::new(&dup), dup.len() as u64, NORMAL_SIZE).is_err());
        // Corrupted text CRC.
        let mut bad = ok.clone();
        let text = bad.windows(4).position(|w| w == b"tEXt").unwrap();
        bad[text + 6] ^= 1;
        assert!(read_meta(&mut Cursor::new(&bad), bad.len() as u64, NORMAL_SIZE).is_err());
        // Non-numeric MTime.
        let mtime = png_with(&[(b"tEXt", b"Thumb::MTime\x0012x".to_vec())], 8);
        assert!(read_meta(&mut Cursor::new(&mtime), mtime.len() as u64, NORMAL_SIZE).is_err());
        // An oversized text chunk is skipped unread: no URI, so no match.
        let mut big = b"Thumb::URI\0file:///x".to_vec();
        big.resize(MAX_TEXT_CHUNK as usize + 1, b'a');
        let skipped = png_with(&[(b"tEXt", big)], 8);
        let meta = read_meta(
            &mut Cursor::new(&skipped),
            skipped.len() as u64,
            NORMAL_SIZE,
        )
        .unwrap();
        assert!(meta.uri.is_none());
        // Uncompressed iTXt is understood; compressed iTXt is ignored.
        let itxt = png_with(&[(b"iTXt", b"Thumb::URI\0\0\0en\0\0file:///i".to_vec())], 8);
        let meta = read_meta(&mut Cursor::new(&itxt), itxt.len() as u64, NORMAL_SIZE).unwrap();
        assert_eq!(meta.uri.as_deref(), Some(&b"file:///i"[..]));
        let ztxt = png_with(&[(b"iTXt", b"Thumb::URI\0\x01\0\0\0xx".to_vec())], 8);
        let meta = read_meta(&mut Cursor::new(&ztxt), ztxt.len() as u64, NORMAL_SIZE).unwrap();
        assert!(meta.uri.is_none());
    }

    #[test]
    fn fit_rgb_box_filters_down_and_never_upscales() {
        let (w, h, out) = fit_rgb(2, 1, &[1, 2, 3, 4, 5, 6], 128);
        assert_eq!((w, h, out), (2, 1, vec![1, 2, 3, 4, 5, 6]));
        let rgb: Vec<u8> = (0..160 * 120).flat_map(|_| [200u8, 100, 50]).collect();
        let (w, h, out) = fit_rgb(160, 120, &rgb, 128);
        assert_eq!((w, h), (128, 96));
        assert_eq!(out.len(), 128 * 96 * 3);
        assert!(out.as_chunks::<3>().0.iter().all(|p| p == &[200, 100, 50]));
        let (w, h, _) = fit_rgb(160, 1, &vec![0; 160 * 3], 128);
        assert_eq!((w, h), (128, 1));
    }
}
