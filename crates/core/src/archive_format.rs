//! Archive formats recognised by name and verified by magic bytes before any
//! external reader is started. Pure, cross-platform; no filesystem access.
use std::ffi::{OsStr, OsString};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar(Compression),
    SevenZip,
    /// Read-only: libarchive extracts RAR/RAR5 but never writes it.
    Rar,
    /// One compressed file (not a container), e.g. `notes.txt.xz`.
    Single(Compression),
}

const SUFFIXES: &[(&str, Format)] = &[
    (".tar.gz", Format::Tar(Compression::Gzip)),
    (".tgz", Format::Tar(Compression::Gzip)),
    (".tar.bz2", Format::Tar(Compression::Bzip2)),
    (".tbz2", Format::Tar(Compression::Bzip2)),
    (".tbz", Format::Tar(Compression::Bzip2)),
    (".tar.xz", Format::Tar(Compression::Xz)),
    (".txz", Format::Tar(Compression::Xz)),
    (".tar.zst", Format::Tar(Compression::Zstd)),
    (".tzst", Format::Tar(Compression::Zstd)),
    (".tar", Format::Tar(Compression::None)),
    (".zip", Format::Zip),
    (".7z", Format::SevenZip),
    (".rar", Format::Rar),
    (".gz", Format::Single(Compression::Gzip)),
    (".bz2", Format::Single(Compression::Bzip2)),
    (".xz", Format::Single(Compression::Xz)),
    (".zst", Format::Single(Compression::Zstd)),
];

fn suffix(name: &OsStr) -> Option<(Format, usize)> {
    let raw = name.as_encoded_bytes();
    SUFFIXES.iter().find_map(|(ext, format)| {
        let ext = ext.as_bytes();
        (raw.len() > ext.len() && raw[raw.len() - ext.len()..].eq_ignore_ascii_case(ext))
            .then_some((*format, ext.len()))
    })
}

impl Format {
    /// Format implied by the final basename; the content is checked separately.
    pub fn from_name(name: &OsStr) -> Option<Self> {
        suffix(name).map(|(format, _)| format)
    }
    /// Basename without the archive suffix (`a.tar.gz` -> `a`).
    pub fn stem(name: &OsStr) -> OsString {
        let raw = name.as_encoded_bytes();
        let cut = suffix(name).map_or(0, |(_, n)| n);
        let stem = &raw[..raw.len() - cut];
        if stem.is_empty() || stem == b"." || stem == b".." {
            return "archivo".into();
        }
        // SAFETY: the cut is at an ASCII suffix boundary of an OsStr encoding.
        unsafe { OsStr::from_encoded_bytes_unchecked(stem) }.to_os_string()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "ZIP",
            Self::Tar(Compression::None) => "tar",
            Self::Tar(Compression::Gzip) => "tar.gz",
            Self::Tar(Compression::Bzip2) => "tar.bz2",
            Self::Tar(Compression::Xz) => "tar.xz",
            Self::Tar(Compression::Zstd) => "tar.zst",
            Self::SevenZip => "7z",
            Self::Rar => "RAR",
            Self::Single(Compression::Gzip) => "gzip",
            Self::Single(Compression::Bzip2) => "bzip2",
            Self::Single(Compression::Xz) => "xz",
            Self::Single(Compression::Zstd) => "zstd",
            Self::Single(Compression::None) => "sin compresión",
        }
    }
    /// The first bytes must match the declared container or compressor.
    /// Plain `.tar` requires the POSIX/GNU `ustar` magic at offset 257.
    pub fn magic_matches(self, head: &[u8]) -> bool {
        let compressed = |c: Compression| match c {
            Compression::Gzip => head.starts_with(&[0x1f, 0x8b]),
            Compression::Bzip2 => head.starts_with(b"BZh"),
            Compression::Xz => head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]),
            Compression::Zstd => head.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]),
            Compression::None => head.get(257..262) == Some(b"ustar"),
        };
        match self {
            Self::Zip => {
                head.starts_with(b"PK\x03\x04")
                    || head.starts_with(b"PK\x05\x06")
                    || head.starts_with(b"PK\x07\x08")
            }
            Self::SevenZip => head.starts_with(&[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c]),
            Self::Rar => head.starts_with(b"Rar!\x1a\x07"),
            Self::Tar(c) => compressed(c),
            Self::Single(Compression::None) => false,
            Self::Single(c) => compressed(c),
        }
    }
    /// libarchive's reported reader must belong to the declared family. This
    /// keeps mtree/cpio/iso and other readers (mtree can reference host files)
    /// out of the pipeline even if a compressed stream wraps them.
    pub fn reader_matches(self, reported: &str) -> bool {
        let format = reported
            .strip_prefix("Archive Format: ")
            .unwrap_or(reported)
            .split(",  Compression:")
            .next()
            .unwrap_or("");
        match self {
            Self::Tar(_) => {
                format.starts_with("POSIX ustar")
                    || format.starts_with("POSIX pax")
                    || format.starts_with("GNU tar")
                    || format.starts_with("tar")
                    || format.starts_with("Solaris tar")
                    || format.starts_with("SUN tar")
            }
            Self::Zip => format.starts_with("ZIP"),
            Self::SevenZip => format.starts_with("7-Zip"),
            Self::Rar => format.starts_with("RAR"),
            Self::Single(_) => false,
        }
    }
}

/// Recognised archive basename; used to route previews and extraction.
pub fn is_archive_name(name: &OsStr) -> bool {
    Format::from_name(name).is_some()
}

/// Writable formats offered by the compress dialog, in cycle order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateFormat {
    Zip,
    TarXz,
    TarGz,
    TarZst,
    SevenZip,
}
impl CreateFormat {
    pub const ALL: [Self; 5] = [
        Self::Zip,
        Self::TarXz,
        Self::TarGz,
        Self::TarZst,
        Self::SevenZip,
    ];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => ".zip",
            Self::TarXz => ".tar.xz",
            Self::TarGz => ".tar.gz",
            Self::TarZst => ".tar.zst",
            Self::SevenZip => ".7z",
        }
    }
    pub fn from_name(name: &OsStr) -> Option<Self> {
        match Format::from_name(name)? {
            Format::Zip => Some(Self::Zip),
            Format::Tar(Compression::Xz) => Some(Self::TarXz),
            Format::Tar(Compression::Gzip) => Some(Self::TarGz),
            Format::Tar(Compression::Zstd) => Some(Self::TarZst),
            Format::SevenZip => Some(Self::SevenZip),
            _ => None,
        }
    }
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
    /// Replace a recognised archive suffix (or append) with this format's.
    pub fn rename(self, name: &OsStr) -> OsString {
        let mut stem = if Format::from_name(name).is_some() {
            Format::stem(name)
        } else {
            name.to_os_string()
        };
        stem.push(self.extension());
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_map_to_formats_and_stems() {
        for (name, format, stem) in [
            ("a.tar.gz", Format::Tar(Compression::Gzip), "a"),
            ("A.TGZ", Format::Tar(Compression::Gzip), "A"),
            ("b.tar.xz", Format::Tar(Compression::Xz), "b"),
            ("c.tar.zst", Format::Tar(Compression::Zstd), "c"),
            ("d.tar.bz2", Format::Tar(Compression::Bzip2), "d"),
            ("e.tar", Format::Tar(Compression::None), "e"),
            ("f.7z", Format::SevenZip, "f"),
            ("g.rar", Format::Rar, "g"),
            ("h.zip", Format::Zip, "h"),
            ("notes.txt.xz", Format::Single(Compression::Xz), "notes.txt"),
            ("i.gz", Format::Single(Compression::Gzip), "i"),
            ("j.zst", Format::Single(Compression::Zstd), "j"),
        ] {
            assert_eq!(Format::from_name(OsStr::new(name)), Some(format), "{name}");
            assert_eq!(Format::stem(OsStr::new(name)), OsString::from(stem));
        }
        assert_eq!(Format::from_name(OsStr::new(".tar")), None);
        assert_eq!(Format::from_name(OsStr::new("x.txt")), None);
        assert_eq!(Format::stem(OsStr::new("..tar")), OsString::from("archivo"));
    }
    #[test]
    fn magic_and_reader_must_match_the_family() {
        assert!(Format::SevenZip.magic_matches(&[b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c, 0]));
        assert!(!Format::Zip.magic_matches(b"Rar!\x1a\x07\x00"));
        let mut tar = vec![0u8; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert!(Format::Tar(Compression::None).magic_matches(&tar));
        assert!(!Format::Tar(Compression::None).magic_matches(b"./x type=file"));
        assert!(
            Format::Tar(Compression::Xz)
                .reader_matches("Archive Format: POSIX pax interchange format,  Compression: xz")
        );
        assert!(
            !Format::Tar(Compression::Gzip)
                .reader_matches("Archive Format: mtree,  Compression: gzip")
        );
        assert!(!Format::Zip.reader_matches("Archive Format: 7-Zip,  Compression: none"));
    }
    #[test]
    fn create_formats_cycle_and_rename() {
        let mut f = CreateFormat::Zip;
        let mut seen = vec![];
        for _ in 0..5 {
            seen.push(f.extension());
            f = f.next();
        }
        assert_eq!(f, CreateFormat::Zip);
        assert_eq!(seen, [".zip", ".tar.xz", ".tar.gz", ".tar.zst", ".7z"]);
        assert_eq!(
            CreateFormat::TarXz.rename(OsStr::new("Fotos.zip")),
            OsString::from("Fotos.tar.xz")
        );
        assert_eq!(
            CreateFormat::SevenZip.rename(OsStr::new("Fotos")),
            OsString::from("Fotos.7z")
        );
        assert_eq!(CreateFormat::from_name(OsStr::new("x.rar")), None);
    }
}
