//! External-tool archive tests. Fixtures are built at test time with bsdtar
//! itself inside a fresh private directory under `TMPDIR`; nothing is deleted
//! and nothing outside that directory is touched. Skips when bsdtar is absent.
use crate::{
    archive_bsdtar::{self as tool, Kind, Top},
    archive_bsdtar_create as create, archive_bsdtar_extract as extract,
    archive_format::{CreateFormat, Format},
    copy_engine::Control,
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::{ffi::OsStrExt, fs::DirBuilderExt},
    path::{Path, PathBuf},
    process::Command,
};

fn bsdtar() -> Option<PathBuf> {
    match tool::program("bsdtar") {
        Ok(p) => Some(p),
        Err(_) => {
            eprintln!("SKIP: bsdtar no está instalado; pruebas de formatos externos omitidas");
            None
        }
    }
}

/// Fresh private 0700 directory for one test; retained after the run.
fn private(test: &str) -> PathBuf {
    let base = std::env::temp_dir();
    assert!(base.is_absolute());
    let path = base.join(format!(
        "files-bsdtar-{test}-{}-{}",
        std::process::id(),
        tool::random_hex().unwrap()
    ));
    fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
    // Reject a symlinked base: the directory must be exactly what we created.
    assert!(fs::symlink_metadata(&path).unwrap().is_dir());
    path
}

fn run(program: &Path, cwd: &Path, args: &[&OsStr]) {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .status()
        .unwrap();
    assert!(status.success(), "fixture command failed: {args:?}");
}

fn tree(root: &Path) -> PathBuf {
    let src = root.join("src");
    fs::create_dir_all(src.join("proj/sub")).unwrap();
    fs::write(src.join("proj/a.txt"), b"hola").unwrap();
    fs::write(src.join("proj/sub/b.bin"), [0u8, 1, 2, 255]).unwrap();
    fs::write(src.join("proj/-dash"), b"d").unwrap();
    fs::write(src.join("proj/nl\nname"), b"n").unwrap();
    fs::write(src.join("proj/niño €.txt"), b"utf8").unwrap();
    std::os::unix::fs::symlink("a.txt", src.join("proj/link")).unwrap();
    fs::write(src.join("loose.txt"), b"suelto").unwrap();
    src
}

fn dest(root: &Path, name: &str) -> PathBuf {
    let d = root.join(name);
    fs::create_dir(&d).unwrap();
    d
}

fn names(dir: &Path) -> Vec<OsString> {
    let mut v: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    v.sort();
    v
}

fn make(src: &Path, roots: &[&str], out_dir: &Path, name: &str) -> PathBuf {
    let roots: Vec<OsString> = roots.iter().map(OsString::from).collect();
    let prepared =
        create::Prepared::capture(src, &roots, out_dir, OsStr::new(name), &Control::default())
            .unwrap();
    prepared
        .execute(&Control::default(), |_, _| {})
        .unwrap()
        .destination
}

fn extract_into(archive: &Path, out: &Path, name: &str) -> Result<extract::Done, String> {
    let prepared = extract::Prepared::capture(archive, out, OsStr::new(name), &Control::default())
        .map_err(|e| e.to_string())?;
    prepared
        .execute(&Control::default(), |_, _| {})
        .map_err(|e| e.to_string())
}

#[test]
fn round_trip_create_list_extract_for_each_writable_format() {
    if bsdtar().is_none() {
        return;
    }
    let root = private("roundtrip");
    let src = tree(&root);
    for format in [
        CreateFormat::TarXz,
        CreateFormat::TarGz,
        CreateFormat::TarZst,
        CreateFormat::SevenZip,
    ] {
        let out = dest(&root, &format!("out{}", format.extension()));
        let archive = make(
            &src,
            &["proj"],
            &out,
            &format!("proj{}", format.extension()),
        );
        let listing = tool::list(&archive, &Control::default(), 100).unwrap();
        assert!(listing.require_safe().is_ok(), "{format:?}");
        assert_eq!(
            listing.top,
            Top::Single {
                name: b"proj".to_vec(),
                directory: true
            }
        );
        assert!(
            listing
                .rows
                .iter()
                .any(|r| r.name == b"proj/nl\nname" && r.kind == Kind::File)
        );
        let link = listing
            .rows
            .iter()
            .find(|r| r.name == b"proj/link")
            .expect("link listed");
        assert_eq!(link.kind, Kind::Symlink, "{format:?}");
        let x = dest(&root, &format!("x{}", format.extension()));
        let done = extract_into(&archive, &x, "proj").unwrap();
        assert_eq!(done.destination, x.join("proj"));
        assert!(!done.renamed);
        assert_eq!(fs::read(x.join("proj/a.txt")).unwrap(), b"hola");
        assert_eq!(
            fs::read(x.join("proj/sub/b.bin")).unwrap(),
            [0u8, 1, 2, 255]
        );
        assert_eq!(fs::read(x.join("proj/-dash")).unwrap(), b"d");
        assert_eq!(fs::read(x.join("proj/nl\nname")).unwrap(), b"n");
        assert_eq!(
            fs::read(x.join("proj/niño €.txt")).unwrap(),
            b"utf8",
            "{format:?}"
        );
        let meta = fs::symlink_metadata(x.join("proj/link")).unwrap();
        assert!(meta.file_type().is_symlink(), "{format:?} keeps the link");
        assert_eq!(
            fs::read_link(x.join("proj/link")).unwrap(),
            Path::new("a.txt")
        );
        // Only the published folder remains: no staging leftover.
        assert_eq!(names(&x), vec![OsString::from("proj")]);
        // The source tree is unchanged.
        assert_eq!(fs::read(src.join("proj/a.txt")).unwrap(), b"hola");
    }
}

#[test]
fn several_top_level_entries_go_into_a_folder_named_after_the_archive() {
    if bsdtar().is_none() {
        return;
    }
    let root = private("multiroot");
    let src = tree(&root);
    let out = dest(&root, "out");
    let archive = make(&src, &["proj", "loose.txt"], &out, "Varios.tar.gz");
    let x = dest(&root, "x");
    let done = extract_into(&archive, &x, "Varios").unwrap();
    assert_eq!(done.destination, x.join("Varios"));
    assert_eq!(
        names(&x.join("Varios")),
        vec![OsString::from("loose.txt"), OsString::from("proj")]
    );
    assert_eq!(names(&x), vec![OsString::from("Varios")]);
}

#[test]
fn bsdtar_reads_zip_and_single_compressed_files() {
    let Some(bsdtar) = bsdtar() else {
        return;
    };
    let root = private("zipsingle");
    let src = tree(&root);
    let out = dest(&root, "out");
    run(
        &bsdtar,
        &src,
        &[
            OsStr::new("-c"),
            OsStr::new("--format"),
            OsStr::new("zip"),
            OsStr::new("-f"),
            out.join("p.zip").as_os_str(),
            OsStr::new("proj"),
        ],
    );
    let x = dest(&root, "x");
    let done = extract_into(&out.join("p.zip"), &x, "p").unwrap();
    assert_eq!(fs::read(done.destination.join("a.txt")).unwrap(), b"hola");
    for (tool_name, ext) in [
        ("xz", "xz"),
        ("gzip", "gz"),
        ("zstd", "zst"),
        ("bzip2", "bz2"),
    ] {
        let Ok(program) = tool::program(tool_name) else {
            eprintln!("SKIP: falta {tool_name}");
            continue;
        };
        let plain = out.join(format!("notes-{ext}.txt"));
        fs::write(&plain, b"texto plano\n").unwrap();
        let compressed = out.join(format!("notes-{ext}.txt.{ext}"));
        let file = fs::File::create(&compressed).unwrap();
        let status = Command::new(program)
            .arg("-c")
            .arg(&plain)
            .stdout(file)
            .status()
            .unwrap();
        assert!(status.success());
        let x = dest(&root, &format!("single-{ext}"));
        let done = extract_into(&compressed, &x, "ignored").unwrap();
        assert_eq!(done.destination, x.join(format!("notes-{ext}.txt")));
        assert_eq!(fs::read(&done.destination).unwrap(), b"texto plano\n");
    }
}

#[test]
fn created_archives_store_symlinks_as_links_and_never_follow_them() {
    if bsdtar().is_none() {
        return;
    }
    let root = private("nofollow");
    let src = root.join("src");
    fs::create_dir_all(src.join("proj")).unwrap();
    fs::create_dir_all(src.join("outside")).unwrap();
    fs::write(src.join("outside/secret"), b"no").unwrap();
    std::os::unix::fs::symlink("../outside", src.join("proj/out")).unwrap();
    fs::write(src.join("proj/f"), b"f").unwrap();
    let out = dest(&root, "out");
    for format in [CreateFormat::TarXz, CreateFormat::SevenZip] {
        let archive = make(&src, &["proj"], &out, &format!("p{}", format.extension()));
        let listing = tool::list(&archive, &Control::default(), 100).unwrap();
        assert!(listing.rows.iter().all(|r| !r.name.ends_with(b"secret")));
        let link = listing.rows.iter().find(|r| r.name == b"proj/out").unwrap();
        assert_eq!(link.kind, Kind::Symlink);
        assert_eq!(link.target.as_deref(), Some(&b"../outside"[..]));
        // Extracting it is refused: once `proj` is published the link would
        // point outside it. The staging folder is kept as failed evidence.
        let x = dest(&root, &format!("x{}", format.extension()));
        let e = extract_into(&archive, &x, "p").unwrap_err();
        assert!(e.contains("fuera de la carpeta"), "{e}");
        assert!(e.contains(".files-extract-failed-"), "{e}");
        assert!(
            names(&x)
                .iter()
                .all(|n| n.as_bytes().starts_with(b".files-extract-failed-"))
        );
    }
}

fn hostile(bsdtar: &Path, root: &Path, name: &str, args: &[&OsStr]) -> PathBuf {
    let work = root.join(format!("fx-{name}"));
    fs::create_dir(&work).unwrap();
    fs::write(work.join("f"), b"evil").unwrap();
    let archive = root.join(name);
    let mut full: Vec<&OsStr> = vec![OsStr::new("-c"), OsStr::new("-f"), archive.as_os_str()];
    full.extend_from_slice(args);
    run(bsdtar, &work, &full);
    archive
}

#[test]
fn hostile_archives_are_rejected_and_nothing_is_published() {
    let Some(bsdtar) = bsdtar() else {
        return;
    };
    let root = private("hostile");
    let s = |v: &'static str| OsStr::new(v);
    let absolute = hostile(
        &bsdtar,
        &root,
        "abs.tar",
        &[s("-P"), s("-s"), s("|^f$|/tmp/files-bsdtar-never|"), s("f")],
    );
    let traversal = hostile(
        &bsdtar,
        &root,
        "trav.tar",
        &[s("-s"), s("|^f$|../../escape|"), s("f")],
    );
    // A symlink escaping the root followed by a file written through it.
    let work = root.join("fx-escape");
    fs::create_dir_all(work.join("first")).unwrap();
    fs::create_dir_all(work.join("second/l")).unwrap();
    std::os::unix::fs::symlink("../../outside", work.join("first/l")).unwrap();
    fs::write(work.join("second/l/evil"), b"evil").unwrap();
    let escape = root.join("escape.tar");
    run(
        &bsdtar,
        &work,
        &[
            s("-c"),
            s("-f"),
            escape.as_os_str(),
            s("-C"),
            s("first"),
            s("l"),
            s("-C"),
            s("../second"),
            s("l/evil"),
        ],
    );
    let absolute_link = root.join("abslink.tar");
    let work = root.join("fx-abslink");
    fs::create_dir(&work).unwrap();
    std::os::unix::fs::symlink("/etc", work.join("etc")).unwrap();
    run(
        &bsdtar,
        &work,
        &[s("-c"), s("-f"), absolute_link.as_os_str(), s("etc")],
    );
    // Hardlink member.
    let work = root.join("fx-hard");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("a"), b"a").unwrap();
    fs::hard_link(work.join("a"), work.join("b")).unwrap();
    let hard = root.join("hard.tar");
    run(
        &bsdtar,
        &work,
        &[s("-c"), s("-f"), hard.as_os_str(), s("a"), s("b")],
    );
    // FIFO member (creatable without root).
    let work = root.join("fx-fifo");
    fs::create_dir(&work).unwrap();
    let fifo = std::ffi::CString::new(work.join("p").as_os_str().as_bytes()).unwrap();
    let fifo_ok = unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) } == 0;
    let fifo_tar = root.join("fifo.tar");
    if fifo_ok {
        run(
            &bsdtar,
            &work,
            &[s("-c"), s("-f"), fifo_tar.as_os_str(), s("p")],
        );
    }
    let mut cases = vec![
        (absolute, "ruta absoluta"),
        (traversal, "«..»"),
        (escape, "sale del archivo"),
        (absolute_link, "enlace simbólico absoluto"),
        (hard, "enlace duro"),
    ];
    if fifo_ok {
        cases.push((fifo_tar, "FIFO"));
    }
    for (archive, expected) in cases {
        let x = dest(
            &root,
            &format!("x-{}", archive.file_name().unwrap().to_string_lossy()),
        );
        let error = extract_into(&archive, &x, "out").unwrap_err();
        assert!(error.contains(expected), "{archive:?}: {error}");
        assert!(names(&x).is_empty(), "nothing created for {archive:?}");
    }
    assert!(!Path::new("/tmp/files-bsdtar-never").exists());
    assert!(!root.join("outside").exists());
}

#[test]
fn mtree_or_foreign_content_inside_a_compressed_tar_is_refused() {
    let Some(_) = bsdtar() else {
        return;
    };
    let Ok(gzip) = tool::program("gzip") else {
        eprintln!("SKIP: falta gzip");
        return;
    };
    let root = private("mtree");
    let spec = root.join("spec");
    fs::write(&spec, b"#mtree\n./x type=file contents=/etc/hostname\n").unwrap();
    let evil = root.join("evil.tar.gz");
    let status = Command::new(gzip)
        .arg("-c")
        .arg(&spec)
        .stdout(fs::File::create(&evil).unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    let x = dest(&root, "x");
    let error = extract_into(&evil, &x, "out").unwrap_err();
    assert!(error.contains("Formato interno no admitido"), "{error}");
    assert!(names(&x).is_empty());
    // Wrong magic for the extension never reaches a reader.
    let fake = root.join("fake.7z");
    fs::write(&fake, b"not an archive").unwrap();
    let error = extract_into(&fake, &x, "out").unwrap_err();
    assert!(error.contains("no corresponde"), "{error}");
}

#[test]
fn encrypted_archives_are_reported_without_prompting() {
    let Some(bsdtar) = bsdtar() else {
        return;
    };
    let root = private("encrypted");
    let src = tree(&root);
    for (name, method) in [
        ("aes.zip", "zip:encryption=aes256"),
        ("zc.zip", "zip:encryption=zipcrypt"),
    ] {
        let archive = root.join(name);
        run(
            &bsdtar,
            &src,
            &[
                OsStr::new("-c"),
                OsStr::new("--format"),
                OsStr::new("zip"),
                OsStr::new("--options"),
                OsStr::new(method),
                OsStr::new("--passphrase"),
                OsStr::new("secreto"),
                OsStr::new("-f"),
                archive.as_os_str(),
                OsStr::new("loose.txt"),
            ],
        );
        let x = dest(&root, &format!("x-{name}"));
        let error = extract_into(&archive, &x, "out").unwrap_err();
        assert!(error.contains(tool::ENCRYPTED), "{name}: {error}");
        assert!(
            names(&x)
                .iter()
                .all(|n| n.as_bytes().starts_with(b".files-extract-failed-")),
            "{name}: nothing published"
        );
    }
}

#[test]
fn existing_destination_names_are_never_overwritten() {
    if bsdtar().is_none() {
        return;
    }
    let root = private("noclobber");
    let src = tree(&root);
    let out = dest(&root, "out");
    let archive = make(&src, &["proj"], &out, "proj.tar.zst");
    let x = dest(&root, "x");
    fs::create_dir(x.join("proj")).unwrap();
    fs::write(x.join("proj/keep"), b"original").unwrap();
    let done = extract_into(&archive, &x, "proj").unwrap();
    assert!(done.renamed);
    assert_eq!(done.destination, x.join("proj (2)"));
    assert_eq!(names(&x.join("proj")), vec![OsString::from("keep")]);
    assert_eq!(fs::read(x.join("proj/keep")).unwrap(), b"original");
    // Creation never replaces an existing archive either.
    fs::write(out.join("again.tar.xz"), b"keep me").unwrap();
    let created = make(&src, &["proj"], &out, "again.tar.xz");
    assert_eq!(created, out.join("again (2).tar.xz"));
    assert_eq!(fs::read(out.join("again.tar.xz")).unwrap(), b"keep me");
    assert!(
        names(&out)
            .iter()
            .all(|n| !n.as_bytes().starts_with(b".files-create-"))
    );
}

#[test]
fn cancellation_publishes_nothing_and_kills_the_child() {
    if bsdtar().is_none() {
        return;
    }
    let root = private("cancel");
    let src = tree(&root);
    let out = dest(&root, "out");
    let archive = make(&src, &["proj"], &out, "proj.tar.gz");
    let x = dest(&root, "x");
    let prepared =
        extract::Prepared::capture(&archive, &x, OsStr::new("proj"), &Control::default()).unwrap();
    let control = Control::default();
    control.cancel();
    let failure = prepared.execute(&control, |_, _| {}).unwrap_err();
    assert!(failure.cancelled);
    assert!(names(&x).is_empty());
    // Creation cancelled before publication leaves no archive behind.
    let roots = [OsString::from("proj")];
    let prepared = create::Prepared::capture(
        &src,
        &roots,
        &out,
        OsStr::new("c.tar.xz"),
        &Control::default(),
    )
    .unwrap();
    let failure = prepared.execute(&control, |_, _| {}).unwrap_err();
    assert!(failure.cancelled);
    assert!(!out.join("c.tar.xz").exists());
    // A running child is killed promptly on cancellation.
    let Ok(sleep) = tool::program("sleep") else {
        return;
    };
    let control = Control::default();
    let remote = control.clone();
    let started = std::time::Instant::now();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        remote.cancel();
    });
    let result = tool::supervise(
        tool::Spec {
            program: &sleep,
            args: vec!["30".into()],
            cwd: &root,
            stdin: tool::Input::Null,
            stdout: tool::Output::Null,
        },
        &control,
        |_, _, _| Ok(()),
        || Ok(()),
    );
    canceller.join().unwrap();
    assert_eq!(
        result.err().map(|e| e.kind()),
        Some(std::io::ErrorKind::Interrupted)
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn entry_count_budget_is_enforced_in_listing_and_staged_tree() {
    let Some(bsdtar) = bsdtar() else {
        return;
    };
    let root = private("budget");
    let work = root.join("many");
    fs::create_dir(&work).unwrap();
    for i in 0..10 {
        fs::write(work.join(format!("f{i}")), b"x").unwrap();
    }
    let archive = root.join("many.tar");
    run(
        &bsdtar,
        &root,
        &[
            OsStr::new("-c"),
            OsStr::new("-f"),
            archive.as_os_str(),
            OsStr::new("many"),
        ],
    );
    let (file, format, _) = tool::open_source(&archive).unwrap();
    assert_eq!(
        format,
        Format::Tar(crate::archive_format::Compression::None)
    );
    let error = tool::list_limited(file, format, &archive, &Control::default(), 0, 5).unwrap_err();
    assert!(error.to_string().contains("más de 5 entradas"), "{error}");
    let error = tool::verify_tree(&work, u64::MAX, 5, &Control::default()).unwrap_err();
    assert!(error.to_string().contains("supera 5 entradas"));
    let error = tool::verify_tree(&work, 3, 100, &Control::default()).unwrap_err();
    assert!(error.to_string().contains("espacio presupuestado"));
    assert_eq!(names(&work).len(), 10, "verification never deletes");
}

#[test]
fn staged_tree_verification_rejects_links_escaping_through_chains() {
    let root = private("verify");
    let staged = root.join("stage");
    fs::create_dir_all(staged.join("x")).unwrap();
    // `a` -> "." is confined, but `x/y` -> "../a/.." resolves above the root.
    std::os::unix::fs::symlink(".", staged.join("a")).unwrap();
    std::os::unix::fs::symlink("../a/..", staged.join("x/y")).unwrap();
    let error = tool::verify_tree(&staged, u64::MAX, 100, &Control::default()).unwrap_err();
    assert!(error.to_string().contains("fuera de la carpeta"), "{error}");
    // A lexically harmless dangling link inside the root is accepted.
    let ok = root.join("ok");
    fs::create_dir_all(ok.join("d")).unwrap();
    std::os::unix::fs::symlink("../missing/file", ok.join("d/l")).unwrap();
    std::os::unix::fs::symlink("d", ok.join("dl")).unwrap();
    let totals = tool::verify_tree(&ok, u64::MAX, 100, &Control::default()).unwrap();
    assert_eq!((totals.symlinks, totals.directories), (2, 1));
    // Hardlinks in the staged tree are rejected.
    let hard = root.join("hard");
    fs::create_dir(&hard).unwrap();
    fs::write(hard.join("a"), b"a").unwrap();
    fs::hard_link(hard.join("a"), hard.join("b")).unwrap();
    let error = tool::verify_tree(&hard, u64::MAX, 100, &Control::default()).unwrap_err();
    assert!(error.to_string().contains("Enlace duro"));
}

#[test]
fn creation_review_rejects_output_inside_selection_and_hardlinks_for_tar() {
    if bsdtar().is_none() {
        return;
    }
    let root = private("createreview");
    let src = tree(&root);
    let roots = [OsString::from("proj")];
    let inside = create::Prepared::capture(
        &src,
        &roots,
        &src.join("proj/sub"),
        OsStr::new("x.tar.xz"),
        &Control::default(),
    )
    .unwrap_err();
    assert!(inside.to_string().contains("dentro de la selección"));
    fs::hard_link(src.join("proj/a.txt"), src.join("proj/a-hard")).unwrap();
    let out = dest(&root, "out");
    let hard = create::Prepared::capture(
        &src,
        &roots,
        &out,
        OsStr::new("x.tar.gz"),
        &Control::default(),
    )
    .unwrap_err();
    assert!(hard.to_string().contains("enlaces duros"));
    // 7z stores hardlinked files as independent members and is accepted.
    assert!(
        create::Prepared::capture(&src, &roots, &out, OsStr::new("x.7z"), &Control::default())
            .is_ok()
    );
    let bad =
        create::Prepared::capture(&src, &roots, &out, OsStr::new("x.rar"), &Control::default())
            .unwrap_err();
    assert!(bad.to_string().contains("Formato"));
}
