//! Archive listing text only. ZIP32 uses the native central-directory parser;
//! tar.*, 7z, RAR, ZIP64 and single compressed files are listed by an
//! ephemeral `bsdtar -tvv` child (killed with this helper). No extraction or
//! execution. Listing metadata does not prove payload integrity or safety.
use files_core::archive_list::{self, Entry, Listing};
use std::{io, path::Path};
const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_TEXT: usize = 128 * 1024;
const MAX_LINES: usize = 2000;
const FOOTER_RESERVE: usize = 256;

pub fn load(path: &Path) -> io::Result<crate::text_preview::Preview> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().len() > 4096
        || path.as_os_str().as_encoded_bytes().contains(&0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ruta ZIP no válida",
        ));
    }
    #[cfg(not(target_os = "linux"))]
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Vista ZIP segura pendiente en esta plataforma",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        let metadata = file.metadata()?;
        use files_core::archive_format::Format;
        let format = path.file_name().and_then(Format::from_name);
        let native = format.is_none_or(|f| f == Format::Zip)
            && metadata.is_file()
            && metadata.len() <= MAX_FILE;
        if native {
            // The same opened FD is used throughout. Parent symlinks, mounts and
            // concurrent in-place writes remain outside a snapshot guarantee.
            return match archive_list::read(&mut file) {
                Ok(list) => describe(&list),
                // ZIP64 or unsupported native features: bsdtar lists it.
                Err(native) if format.is_some() => external(path).map_err(|e| {
                    io::Error::new(e.kind(), format!("ZIP nativo: {native} · bsdtar: {e}"))
                }),
                Err(native) => Err(native),
            };
        }
        if format.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ZIP debe ser archivo regular de hasta 8GiB",
            ));
        }
        drop(file);
        external(path)
    }
}
pub const EXTERNAL_ENCODING: &str = "Archivo · listado bsdtar";
#[cfg(target_os = "linux")]
fn external(path: &Path) -> io::Result<crate::text_preview::Preview> {
    let listing = files_core::archive_bsdtar::list(
        path,
        &files_core::copy_engine::Control::default(),
        MAX_LINES,
    )?;
    describe_external(&listing)
}
fn external_row(row: &files_core::archive_bsdtar::Row) -> String {
    use files_core::archive_bsdtar::{Kind, display};
    let kind = match row.kind {
        Kind::File => "archivo",
        Kind::Directory => "carpeta",
        Kind::Symlink => "enlace",
        Kind::Hardlink => "enlace duro",
        Kind::Special(_) => "especial",
    };
    let target = row
        .target
        .as_ref()
        .map(|t| format!(" → {}", display(t)))
        .unwrap_or_default();
    let problem = row
        .problem
        .map(|p| format!(" · INSEGURO: {p}"))
        .unwrap_or_default();
    format!(
        "{}{target} [{kind}{problem}] · {} B · {}\n",
        display(&row.name),
        row.size,
        row.mode
    )
}
fn describe_external(
    list: &files_core::archive_bsdtar::Listing,
) -> io::Result<crate::text_preview::Preview> {
    use std::fmt::Write;
    let mut text = String::with_capacity(4096);
    let _ = writeln!(
        text,
        "{} · {}\nSin extracción; contenido y sumas NO comprobados.\nNo se ejecutan ni se abren los nombres enumerados.",
        list.format.label(),
        list.reported
    );
    let _ = writeln!(
        text,
        "{} entradas · {} archivos · {} carpetas · {} enlaces · {}{}\n",
        list.entries,
        list.files,
        list.directories,
        list.symlinks,
        list.bytes.map_or_else(
            || "tamaño desconocido".into(),
            |b| format!("{b} B declarados")
        ),
        if list.unsafe_entries > 0 {
            format!(
                " · {} INSEGURAS: la extracción se rechazará",
                list.unsafe_entries
            )
        } else {
            String::new()
        }
    );
    let lines = text.lines().count();
    let mut shown = 0usize;
    for (index, row) in list.rows.iter().enumerate() {
        let row = external_row(row);
        if lines + index + 1 > MAX_LINES - 3
            || text
                .len()
                .checked_add(row.len())
                .is_none_or(|n| n > MAX_TEXT - FOOTER_RESERVE)
        {
            break;
        }
        text.push_str(&row);
        shown = index + 1;
    }
    let total = usize::try_from(list.entries).unwrap_or(usize::MAX);
    let truncated = shown != total;
    let _ = writeln!(
        text,
        "\nEnumeradas {shown} de {total} entradas{}.",
        if truncated {
            " · LISTADO TRUNCADO"
        } else {
            ""
        }
    );
    if text.len() > MAX_TEXT || text.lines().count() > MAX_LINES {
        return Err(io::Error::other("Listado fuera del límite de presentación"));
    }
    Ok(crate::text_preview::Preview {
        relative_source: None,
        text,
        encoding: EXTERNAL_ENCODING,
        truncated,
    })
}
fn row(entry: &Entry) -> String {
    let kind = if entry.symlink {
        "enlace"
    } else if entry.directory {
        "carpeta"
    } else {
        "archivo"
    };
    let mut labels = String::new();
    for (flag, label) in [
        (entry.unsafe_lexical, " · RUTA INSEGURA"),
        (entry.duplicate, " · NOMBRE DUPLICADO"),
        (entry.encrypted, " · cifrado"),
    ] {
        if flag {
            labels.push_str(label);
        }
    }
    let mode = entry
        .unix_mode
        .map(|value| format!(" · modo {value:06o}"))
        .unwrap_or_default();
    format!(
        "{} [{kind}{labels}] · {} B / {} B comprimidos · método {}{mode}\n",
        archive_list::display_name(&entry.name),
        entry.uncompressed_bytes,
        entry.compressed_bytes,
        entry.method
    )
}
fn describe(list: &Listing) -> io::Result<crate::text_preview::Preview> {
    use std::fmt::Write;
    let mut text = String::with_capacity(4096);
    text.push_str("ZIP · listado del directorio central\nSin extracción; datos, registros locales y CRC NO comprobados.\nNo se ejecutan ni se abren los nombres enumerados.\n");
    let _ = writeln!(
        text,
        "{} entradas · {} B declarados sin comprimir · {} B comprimidos\n",
        list.entries.len(),
        list.uncompressed_bytes,
        list.compressed_bytes
    );
    let lines = text.lines().count();
    let mut shown = 0usize;
    for (index, entry) in list.entries.iter().enumerate() {
        let row = row(entry);
        if lines + index + 1 > MAX_LINES - 3
            || text
                .len()
                .checked_add(row.len())
                .is_none_or(|n| n > MAX_TEXT - FOOTER_RESERVE)
        {
            break;
        }
        text.push_str(&row);
        shown = index + 1;
    }
    let truncated = shown != list.entries.len();
    let _ = writeln!(
        text,
        "\nEnumeradas {shown} de {} entradas{}.",
        list.entries.len(),
        if truncated {
            " · LISTADO TRUNCADO"
        } else {
            ""
        }
    );
    if text.len() > MAX_TEXT || text.lines().count() > MAX_LINES {
        return Err(io::Error::other(
            "Listado ZIP fuera del límite de presentación",
        ));
    }
    Ok(crate::text_preview::Preview {
        relative_source: None,
        text,
        encoding: "ZIP · directorio central",
        truncated,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(name: Vec<u8>) -> Entry {
        Entry {
            name,
            compressed_bytes: 1,
            uncompressed_bytes: 20,
            method: 8,
            needed_version: 20,
            flags: 0,
            crc32: 0,
            local_offset: 0,
            encrypted: true,
            utf8_declared: false,
            unix_mode: Some(0o120777),
            directory: false,
            symlink: true,
            unsafe_lexical: true,
            duplicate: true,
        }
    }
    #[test]
    fn safety_labels_raw_names_and_disclaimer_are_visible() {
        let result = describe(&Listing {
            directory_offset: 0,
            entries: vec![entry(b"../raw\xff\n".to_vec())],
            compressed_bytes: 1,
            uncompressed_bytes: 20,
        })
        .unwrap();
        assert!(!result.truncated);
        for expected in [
            "CRC NO comprobados",
            "RUTA INSEGURA",
            "NOMBRE DUPLICADO",
            "cifrado",
            "enlace",
            "\\xFF\\x0A",
            "Enumeradas 1 de 1",
        ] {
            assert!(result.text.contains(expected));
        }
    }
    #[test]
    fn external_listing_labels_unsafe_members_and_links() {
        use files_core::archive_bsdtar::{Kind, Listing, Row, Top};
        let row = |name: &[u8], kind, target: Option<&[u8]>, problem| Row {
            kind,
            name: name.to_vec(),
            size: 3,
            mode: "-rw-r--r--".into(),
            target: target.map(<[u8]>::to_vec),
            problem,
        };
        let list = Listing {
            format: files_core::archive_format::Format::SevenZip,
            reported: "Archive Format: 7-Zip,  Compression: none".into(),
            entries: 3,
            files: 1,
            directories: 0,
            symlinks: 1,
            bytes: Some(3),
            top: Top::Many,
            rows: vec![
                row(b"a\xff", Kind::File, None, None),
                row(
                    b"l",
                    Kind::Symlink,
                    Some(b"../x"),
                    Some("enlace simbólico que sale del archivo"),
                ),
                row(b"/abs", Kind::File, None, Some("ruta absoluta")),
            ],
            unsafe_entries: 2,
            first_problem: None,
        };
        let p = describe_external(&list).unwrap();
        assert_eq!(p.encoding, EXTERNAL_ENCODING);
        assert!(!p.truncated);
        for expected in [
            "7z · Archive Format: 7-Zip",
            "2 INSEGURAS",
            "INSEGURO: ruta absoluta",
            "l → ../x",
            "a\\xFF",
            "Enumeradas 3 de 3",
        ] {
            assert!(p.text.contains(expected), "{expected}\n{}", p.text);
        }
    }
    #[test]
    fn text_and_line_budgets_are_explicit_not_silent() {
        for (count, width) in [(10000, 1), (40, 4096)] {
            let entries = (0..count).map(|_| entry(vec![b'x'; width])).collect();
            let result = describe(&Listing {
                directory_offset: 0,
                entries,
                compressed_bytes: 0,
                uncompressed_bytes: 0,
            })
            .unwrap();
            assert!(result.truncated);
            assert!(result.text.contains("LISTADO TRUNCADO"));
            assert!(result.text.len() <= MAX_TEXT && result.text.lines().count() <= MAX_LINES);
        }
    }
}
