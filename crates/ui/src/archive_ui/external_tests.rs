//! UI flows for bsdtar-backed formats. Fixtures live in a fresh private
//! directory under TMPDIR, are built with bsdtar itself and are never deleted.
use super::*;
use crate::model::Action;
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

fn private() -> Option<PathBuf> {
    if files_core::archive_bsdtar::program("bsdtar").is_err() {
        eprintln!("SKIP: bsdtar no está instalado");
        return None;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "files-ui-bsdtar-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    Some(path)
}
fn wait_model(m: &mut Model, check: impl Fn(&Model) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        m.pump();
        if check(m) {
            return;
        }
        assert!(Instant::now() < end, "{}", m.error);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn select(m: &mut Model, name: &str) {
    wait_model(m, |m| m.loaded);
    let i = m.entries.iter().position(|e| e.name == name).unwrap();
    m.select_entry(i, false, false);
}
fn close(m: &mut Model) {
    m.copy.begin_close();
    let end = Instant::now() + Duration::from_secs(8);
    while !m.copy.is_closed() {
        m.pump();
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn names(dir: &Path) -> Vec<OsString> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    v.sort();
    v
}

#[test]
fn extract_here_reviews_then_publishes_tar_gz_with_a_numbered_name() {
    let Some(root) = private() else {
        return;
    };
    let work = root.join("work");
    fs::create_dir_all(work.join("proj")).unwrap();
    fs::write(work.join("proj/a.txt"), b"hola").unwrap();
    let status = Command::new(files_core::archive_bsdtar::program("bsdtar").unwrap())
        .args(["-c", "-z", "-f"])
        .arg(root.join("proj.tar.gz"))
        .args(["-C"])
        .arg(&work)
        .arg("proj")
        .env("LC_ALL", "C")
        .status()
        .unwrap();
    assert!(status.success());
    // An occupant with the planned name must survive untouched.
    fs::create_dir(root.join("proj")).unwrap();
    fs::write(root.join("proj/keep"), b"original").unwrap();
    let mut m = Model::new(root.clone());
    select(&mut m, "proj.tar.gz");
    m.action(Action::ExtractHere);
    wait_model(&mut m, |m| m.copy_last.is_some());
    let outcome = m.copy_last.as_ref().unwrap();
    assert!(outcome.extracting);
    assert!(
        outcome.result.is_ok(),
        "{:?}",
        outcome.result.as_ref().err().map(|e| e.to_string())
    );
    assert!(
        crate::copy_report::describe(outcome)
            .text
            .contains("nombre numerado")
    );
    assert_eq!(fs::read(root.join("proj (2)/a.txt")).unwrap(), b"hola");
    assert_eq!(fs::read(root.join("proj/keep")).unwrap(), b"original");
    assert_eq!(
        names(&root),
        ["proj", "proj (2)", "proj.tar.gz", "work"].map(OsString::from)
    );
    close(&mut m);
}

#[test]
fn compress_dialog_cycles_format_and_creates_tar_xz() {
    let Some(root) = private() else {
        return;
    };
    fs::create_dir_all(root.join("Fotos/sub")).unwrap();
    fs::write(root.join("Fotos/sub/x.txt"), b"x").unwrap();
    std::os::unix::fs::symlink("sub/x.txt", root.join("Fotos/link")).unwrap();
    let mut m = Model::new(root.clone());
    select(&mut m, "Fotos");
    m.action(Action::CreateArchive);
    let form = m.copy_form.as_ref().unwrap();
    assert!(form.creating);
    assert!(form.destination_path().ends_with("Fotos.zip"));
    // F4 in the compress dialog cycles the format instead of copy conflicts.
    m.action(Action::CopySkipExisting);
    let destination = m.copy_form.as_ref().unwrap().destination_path();
    assert!(destination.ends_with("Fotos.tar.xz"), "{destination:?}");
    m.start_copy();
    wait_model(&mut m, |m| {
        m.copy_form
            .as_ref()
            .is_some_and(|d| !d.preparing && (d.archive.is_some() || !d.error.is_empty()))
    });
    let d = m.copy_form.as_ref().unwrap();
    assert!(d.archive.is_some(), "{}", d.error);
    assert!(d.archive.as_ref().unwrap().summary().contains(".tar.xz"));
    assert!(!destination.exists(), "review is read-only");
    let pair = (d.editor.epoch, d.prepare_id);
    m.action(Action::ApplyArchive(pair.0, pair.1));
    wait_model(&mut m, |m| m.copy_last.is_some());
    let outcome = m.copy_last.as_ref().unwrap();
    assert!(outcome.creating && outcome.result.is_ok());
    let listing = files_core::archive_bsdtar::list(
        &destination,
        &files_core::copy_engine::Control::default(),
        10,
    )
    .unwrap();
    let link = listing
        .rows
        .iter()
        .find(|r| r.name == b"Fotos/link")
        .unwrap();
    assert_eq!(link.kind, files_core::archive_bsdtar::Kind::Symlink);
    close(&mut m);
}
