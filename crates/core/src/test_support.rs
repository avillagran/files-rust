//! Test-only fixture support shared by every crate (`#[path]`-included, never compiled outside
//! `cfg(test)`).
//!
//! Fixtures live in a fresh, exclusive directory under `std::env::temp_dir()` (which honours
//! `TMPDIR`). The directory is created with mode 0700, creation fails if the name exists, and
//! nothing here ever deletes it: the operator disposes of the per-run `TMPDIR`, not the tests.

// Shared by three crates; each one uses only part of it.
#![allow(dead_code)]

use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Creates and returns a new private directory named after `label`, pid, time and a counter.
pub fn private_dir(label: &str) -> PathBuf {
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!(
        "files-test-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap_or_else(|e| panic!("create exclusive fixture {}: {e}", path.display()));
    path
}

/// Asserts that `target` is lexically inside `root` and that no component below `root` is a
/// symlink. Call it before any destructive product call made by a test.
pub fn assert_inside(root: &Path, target: &Path) {
    let rest = target
        .strip_prefix(root)
        .unwrap_or_else(|_| panic!("{} escapes {}", target.display(), root.display()));
    assert!(
        !rest
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "{} contains ..",
        target.display()
    );
    let mut at = root.to_path_buf();
    for part in rest.components() {
        at.push(part);
        if let Ok(meta) = std::fs::symlink_metadata(&at) {
            assert!(
                !meta.file_type().is_symlink(),
                "{} is a symlink",
                at.display()
            );
        }
    }
}

/// Sets the atime of `path` itself (symlinks are not followed) to the year 2100 and leaves the
/// mtime alone, so relatime/strictatime updates on later reads cannot change what captures see.
pub fn pin_future_atime(path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let times = [
        libc::timespec {
            tv_sec: 4_102_444_800,
            tv_nsec: 0,
        },
        libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_OMIT,
        },
    ];
    let rc = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            c.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    assert_eq!(rc, 0, "pin atime of {}", path.display());
}

/// Source patterns that must not appear in any file of the crate (test code included).
const FORBIDDEN: &[&str] = &[
    concat!(".cache/", "files-test"),
    concat!("remove_dir", "_all("),
    concat!("tempfile", "::"),
];

/// Exceptions as `(file relative to src/, line substring, justification)`. Keep empty unless a
/// destructive call is the very behaviour under test, and then keep it inside a private root.
const ALLOW: &[(&str, &str, &str)] = &[];

fn scan(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
        if kind.is_dir() {
            scan(&path, root, out);
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if rel == "test_support.rs" {
                continue;
            }
            for (n, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                for pattern in FORBIDDEN {
                    let allowed = ALLOW
                        .iter()
                        .any(|(f, sub, _why)| *f == rel && line.contains(sub));
                    if line.contains(pattern) && !allowed {
                        out.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                    }
                }
            }
        }
    }
}

/// Guard: fixtures must come from `private_dir` and tests must not delete directory trees.
#[test]
fn sources_have_no_home_fixtures_or_recursive_deletion() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    scan(&root, &root, &mut found);
    assert!(
        found.is_empty(),
        "use test_support::private_dir and do not delete trees:\n{}",
        found.join("\n")
    );
}

#[test]
fn private_dirs_are_exclusive_private_and_under_temp_dir() {
    use std::os::unix::fs::PermissionsExt;
    let a = private_dir("a b");
    let b = private_dir("a b");
    assert_ne!(a, b);
    assert!(a.starts_with(std::env::temp_dir()));
    assert_eq!(
        std::fs::metadata(&a).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_inside(&a, &a.join("x/y"));
}
