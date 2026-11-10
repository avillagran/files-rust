//! Symbolic links to folders browse as folders, under their own path.
//! Fixtures live in a fresh directory under the private test TMPDIR, every
//! link points inside it, and nothing is deleted afterwards.
use crate::{aux_io::AuxIo, model::Model};
use std::{
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn fixture() -> PathBuf {
    static ID: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "files-link-browse-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("real")).unwrap();
    std::fs::write(root.join("real/inside.txt"), b"inside").unwrap();
    std::fs::write(root.join("b-file.txt"), b"file").unwrap();
    symlink("real", root.join("a-link")).unwrap();
    symlink("b-file.txt", root.join("file-link")).unwrap();
    root
}

fn model(root: &Path) -> Model {
    let mut m = Model::new(root.to_path_buf());
    let (tx, _) = mpsc::sync_channel(4);
    let (_, rx) = mpsc::channel();
    m.aux = AuxIo { tx, rx };
    loaded(&mut m);
    m
}

fn loaded(m: &mut Model) {
    let end = Instant::now() + Duration::from_secs(5);
    while !m.loaded {
        m.pump();
        assert!(Instant::now() < end, "listing did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn select(m: &mut Model, name: &str) {
    let i = m
        .entries
        .iter()
        .position(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} listed"));
    m.select_entry(i, false, false);
}

fn order_names(m: &Model) -> Vec<String> {
    m.order
        .iter()
        .map(|&i| m.entries[i].name.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn directory_link_sorts_with_folders_and_stays_a_link() {
    let root = fixture();
    let m = model(&root);
    assert_eq!(
        order_names(&m),
        ["a-link", "real", "b-file.txt", "file-link"]
    );
    let link = m.entries.iter().find(|e| e.name == "a-link").unwrap();
    assert!(link.browses_as_dir() && !link.is_dir());
    // Recursive folder sizing only ever considers real directories.
    let measured: Vec<_> = m.entries.iter().filter(|e| e.is_dir()).collect();
    assert_eq!(measured.len(), 1);
    assert_eq!(measured[0].name, "real");
}

#[test]
fn enter_on_directory_link_navigates_under_the_link_path_and_up_returns() {
    let root = fixture();
    let mut m = model(&root);
    select(&mut m, "a-link");
    m.open_selected();
    assert!(m.open_job.is_none(), "a folder link must not launch an app");
    assert_eq!(m.path, root.join("a-link"));
    loaded(&mut m);
    assert_eq!(order_names(&m), ["inside.txt"]);
    m.navigate(root.clone());
    loaded(&mut m);
    m.pump();
    assert_eq!(m.path, root);
    assert_eq!(
        m.selected.map(|i| m.entries[i].name.clone()),
        Some("a-link".into()),
        "going up focuses the link it came from"
    );
}

#[test]
fn ctrl_enter_opens_directory_link_in_a_tab_and_file_link_is_not_a_folder() {
    let root = fixture();
    let mut m = model(&root);
    select(&mut m, "a-link");
    m.open_focused_in_tab(false);
    assert_eq!(m.tabs.rows.len(), 2);
    assert_eq!(m.tabs.rows[1].path, root.join("a-link"));
    assert_eq!(m.path, root);
    let file_link = m.entries.iter().find(|e| e.name == "file-link").unwrap();
    assert!(!file_link.browses_as_dir());
}
