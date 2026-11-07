//! Bounded sidebar ordering. Keys identify UI rows, never filesystem authority.
use std::path::Path;

pub const MAX_ENTRIES: usize = 128;
pub const MAX_BYTES: usize = 64 * 1024;

/// A lossless, platform-local key for an OS path.
pub fn path_key(section: &str, path: &Path) -> String {
    let mut key = format!("{section}:");
    for byte in path.as_os_str().as_encoded_bytes() {
        use std::fmt::Write;
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// Distinguishes XDG roles which happen to point at the same directory.
pub fn place_key(path: &Path, label: &str) -> String {
    let mut key = path_key("place", path);
    key.push_str("00");
    for byte in label.as_bytes() {
        use std::fmt::Write;
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// Stable order for present rows; keys missing from the preference retain
/// their supplied relative order after the saved keys.
pub fn ordered_indices(keys: &[String], saved: &[String]) -> Vec<usize> {
    let mut indices: Vec<_> = (0..keys.len()).collect();
    indices.sort_by_key(|&i| {
        saved
            .iter()
            .position(|key| key == &keys[i])
            .unwrap_or(usize::MAX)
    });
    indices
}

/// Reorder visible siblings while retaining absent keys and their slots.
/// Return None when the request is stale, ambiguous or exceeds the budget.
pub fn moved(saved: &[String], visible: &[String], key: &str, index: usize) -> Option<Vec<String>> {
    if saved.len() > MAX_ENTRIES
        || visible.len() > MAX_ENTRIES
        || saved.iter().map(String::len).sum::<usize>() > MAX_BYTES
        || saved
            .iter()
            .enumerate()
            .any(|(i, k)| saved[..i].contains(k))
        || visible
            .iter()
            .enumerate()
            .any(|(i, k)| visible[..i].contains(k))
    {
        return None;
    }
    let from = visible.iter().position(|item| item == key)?;
    let mut ordered = visible.to_vec();
    let item = ordered.remove(from);
    ordered.insert(index.min(ordered.len()), item);
    let mut result = saved.to_vec();
    for item in visible {
        if !result.contains(item) {
            result.push(item.clone());
        }
    }
    if result.len() > MAX_ENTRIES || result.iter().map(String::len).sum::<usize>() > MAX_BYTES {
        return None;
    }
    let mut reordered = ordered.into_iter();
    for slot in &mut result {
        if visible.contains(slot) {
            *slot = reordered.next()?;
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_retains_absent_rows_and_other_sections() {
        let saved = ["p:a", "p:missing", "d:x", "p:b"].map(str::to_owned);
        let visible = ["p:a", "p:b", "p:c"].map(str::to_owned);
        let result = moved(&saved, &visible, "p:c", 0).unwrap();
        assert_eq!(result, ["p:c", "p:missing", "d:x", "p:a", "p:b"]);
        assert_eq!(ordered_indices(&visible, &result), [2, 0, 1]);
        assert!(moved(&result, &visible, "p:missing", 0).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn raw_paths_do_not_collapse_to_the_same_key() {
        use std::os::unix::ffi::OsStrExt;
        let a = Path::new(std::ffi::OsStr::from_bytes(b"/\xff"));
        let b = Path::new(std::ffi::OsStr::from_bytes(b"/\xfe"));
        assert_ne!(path_key("place", a), path_key("place", b));
    }
}
