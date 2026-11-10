//! SVG thumbnails through the system's optional `rsvg-convert` (librsvg),
//! a direct child with fixed argv, never a shell and never linked.
//!
//! SVG is hostile input. The verified source descriptor becomes the child's
//! stdin and the argument is `-`, so librsvg has no base URL: it refuses every
//! relative or `file://` reference, and it never fetches network resources.
//! `--unlimited` is NOT passed, keeping librsvg's XML size limits. The child
//! gets rlimits (address space, CPU, descriptors, no core) plus the common
//! supervisor's wall timeout and cancel; its PNG stdout is capped before the
//! existing helper decodes it to 160 px RGB, so no decoder runs in the UI.
use crate::image_preview_job::{Bitmap, THUMBNAIL_DIMENSION};
use files_core::media_source::Revision;
use std::{
    io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub const MAX_SOURCE: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
/// Address-space cap: fontconfig/cairo need about 192 MiB of mappings.
const ADDRESS_SPACE: u64 = 384 * 1024 * 1024;

#[cfg(target_os = "linux")]
pub(crate) fn thumbnail(
    path: &Path,
    expected: Revision,
    cancel: &AtomicBool,
    program: &std::ffi::OsStr,
) -> io::Result<Bitmap> {
    let (source, revision, png) = render(path, expected, cancel, program)?;
    let mut bitmap =
        crate::thumbnail_disk_cache::with_scratch(&png, |scratch, scratch_revision| {
            crate::image_preview_job::thumbnail(scratch.to_path_buf(), scratch_revision, cancel)
        })?;
    revision.revalidate(&source, path)?;
    bitmap.source = Some(revision);
    bitmap.source_width = bitmap.width;
    bitmap.source_height = bitmap.height;
    bitmap.format = "svg".into();
    Ok(bitmap)
}
/// Bounded PNG bytes from rsvg-convert for the verified source revision.
#[cfg(target_os = "linux")]
fn render(
    path: &Path,
    expected: Revision,
    cancel: &AtomicBool,
    program: &std::ffi::OsStr,
) -> io::Result<(std::fs::File, Revision, Vec<u8>)> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    if expected.bytes == 0 || expected.bytes > MAX_SOURCE {
        return Err(io::Error::other(
            "SVG debe ser archivo no vacío de hasta8MiB",
        ));
    }
    let (source, revision) = files_core::media_source::open(path)?;
    if revision != expected {
        return Err(io::Error::other("El SVG cambió antes de renderizar"));
    }
    let fd = source.as_raw_fd();
    let side = THUMBNAIL_DIMENSION.to_string();
    let mut command = std::process::Command::new(program);
    command
        .args([
            "--width",
            &side,
            "--height",
            &side,
            "--keep-aspect-ratio",
            "--format",
            "png",
            "-",
        ])
        .env("LC_ALL", "C");
    // Runs after std's stdio setup: the source replaces the request pipe on
    // stdin. dup2 clears CLOEXEC on fd 0 only; the original stays close-on-exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(fd, 0) < 0 || libc::lseek(0, 0, libc::SEEK_SET) < 0 {
                return Err(io::Error::last_os_error());
            }
            for (resource, value) in [
                (libc::RLIMIT_AS, ADDRESS_SPACE),
                (libc::RLIMIT_CPU, 3),
                (libc::RLIMIT_NOFILE, 64),
                (libc::RLIMIT_CORE, 0),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let png = crate::text_preview_job::run_command_program(
        command,
        &[],
        "thumbnail_svg",
        cancel,
        TIMEOUT,
        crate::thumbnail_disk_cache::SCRATCH_MAX,
        None,
    )
    .map_err(crate::thumbnail_job::tool_error)?;
    if cancel.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "SVG cancelado"));
    }
    revision.revalidate(&source, path)?;
    Ok((source, revision, png))
}
#[cfg(not(target_os = "linux"))]
pub(crate) fn thumbnail(
    _: &Path,
    _: Revision,
    _: &AtomicBool,
    _: &std::ffi::OsStr,
) -> io::Result<Bitmap> {
    Err(crate::thumbnail_job::tool_missing())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use files_core::thumbnail_standard as spec;
    fn installed() -> bool {
        std::process::Command::new("rsvg-convert")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok()
    }
    #[test]
    fn missing_rasterizer_is_reported_as_tool_missing() {
        let dir = crate::thumbnail_disk_cache::retained_test_directory("svg-missing");
        let path = dir.join("a.svg");
        std::fs::write(&path, b"<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let (_, revision) = files_core::media_source::open(&path).unwrap();
        let error = render(
            &path,
            revision,
            &AtomicBool::new(false),
            "files-test-no-such-rsvg-convert".as_ref(),
        )
        .unwrap_err();
        assert!(
            error
                .get_ref()
                .is_some_and(|e| e.to_string().contains("no instalada"))
        );
    }
    #[test]
    fn render_is_bounded_and_never_resolves_external_files() {
        if !installed() {
            eprintln!("rsvg-convert not installed; skipped");
            return;
        }
        let dir = crate::thumbnail_disk_cache::retained_test_directory("svg-render");
        let plain = dir.join("plain.svg");
        std::fs::write(
            &plain,
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200"><rect width="400" height="200" fill="red"/></svg>"#,
        )
        .unwrap();
        let (_, revision) = files_core::media_source::open(&plain).unwrap();
        let (_, _, png) = render(
            &plain,
            revision,
            &AtomicBool::new(false),
            "rsvg-convert".as_ref(),
        )
        .unwrap();
        let meta = spec::read_meta(&mut std::io::Cursor::new(&png), png.len() as u64, 160).unwrap();
        assert_eq!((meta.width, meta.height), (160, 80));
        // A sibling image referenced by relative and absolute file URLs is
        // not loaded: the output equals a document whose image is absent.
        std::fs::write(dir.join("sibling.png"), &png).unwrap();
        let href = format!("file://{}", dir.join("sibling.png").display());
        let with = dir.join("with.svg");
        let without = dir.join("without.svg");
        let doc = |image: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80">{image}</svg>"#
            )
        };
        std::fs::write(
            &with,
            doc(&format!(
                r#"<image href="sibling.png" width="160" height="80"/><image href="{href}" width="160" height="80"/>"#
            )),
        )
        .unwrap();
        std::fs::write(&without, doc("")).unwrap();
        let render_file = |path: &Path| {
            let (_, revision) = files_core::media_source::open(path).unwrap();
            render(
                path,
                revision,
                &AtomicBool::new(false),
                "rsvg-convert".as_ref(),
            )
            .unwrap()
            .2
        };
        assert_eq!(render_file(&with), render_file(&without));
        // A changed source is not rendered under an older revision.
        let (_, old) = files_core::media_source::open(&without).unwrap();
        std::fs::write(&without, doc("<g/>")).unwrap();
        assert!(
            render(
                &without,
                old,
                &AtomicBool::new(false),
                "rsvg-convert".as_ref()
            )
            .is_err()
        );
    }
}
