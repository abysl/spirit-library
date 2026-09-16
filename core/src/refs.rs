use crate::store::{BlobHash, BlobStore};
use std::path::PathBuf;

pub const REFS_DIR: &str = "refs";
pub const MAX_DEPTH: usize = 3;

pub fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('.')
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn valid_name(name: &str) -> bool {
    let segments: Vec<&str> = name.split('/').collect();
    !segments.is_empty() && segments.len() <= MAX_DEPTH && segments.iter().all(|s| valid_segment(s))
}

pub fn dir(store: &BlobStore) -> PathBuf {
    store.root().join(REFS_DIR)
}

pub fn path(store: &BlobStore, name: &str) -> Option<PathBuf> {
    valid_name(name).then(|| {
        name.split('/')
            .fold(dir(store), |path, segment| path.join(segment))
    })
}

pub fn write(store: &BlobStore, name: &str, hash: BlobHash) -> Result<(), String> {
    let path = path(store, name).ok_or_else(|| format!("{name:?} is not a usable ref name"))?;
    let parent = path.parent().ok_or("ref path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{hash}\n")).map_err(|e| e.to_string())
}

pub fn read(store: &BlobStore, name: &str) -> Option<BlobHash> {
    BlobHash::parse(&std::fs::read_to_string(path(store, name)?).ok()?)
}

pub fn remove(store: &BlobStore, name: &str) -> Result<(), String> {
    let path = path(store, name).ok_or_else(|| format!("{name:?} is not a usable ref name"))?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn list(store: &BlobStore) -> Vec<(String, BlobHash)> {
    let mut out = Vec::new();
    walk(&dir(store), None, 1, &mut out);
    out.sort_by(|left, right| left.0.cmp(&right.0));
    out
}

pub fn list_under(store: &BlobStore, prefix: &str) -> Vec<(String, BlobHash)> {
    let head = format!("{prefix}/");
    list(store)
        .into_iter()
        .filter(|(name, _)| name.starts_with(&head))
        .collect()
}

fn walk(
    path: &std::path::Path,
    prefix: Option<&str>,
    depth: usize,
    out: &mut Vec<(String, BlobHash)>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !valid_segment(&file) {
            continue;
        }
        let name = match prefix {
            Some(prefix) => format!("{prefix}/{file}"),
            None => file,
        };
        let path = entry.path();
        if path.is_dir() {
            walk(&path, Some(&name), depth + 1, out);
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(hash) = BlobHash::parse(&text) {
            out.push((name, hash));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> BlobStore {
        let dir = std::env::temp_dir().join(format!("spirit-refs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    #[test]
    fn a_ref_round_trips_through_the_store() {
        let store = scratch("roundtrip");
        let hash = store.put(b"a manifest").unwrap();
        write(&store, "hob", hash).unwrap();
        assert_eq!(read(&store, "hob"), Some(hash));
        remove(&store, "hob").unwrap();
        assert_eq!(read(&store, "hob"), None);
        remove(&store, "hob").unwrap();
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn namespaced_refs_list_alongside_flat_ones() {
        let store = scratch("list");
        let cards = store.put(b"cards").unwrap();
        let plugin = store.put(b"plugin").unwrap();
        write(&store, "hob", cards).unwrap();
        write(&store, "modules/riftbound", plugin).unwrap();
        let listed = list(&store);
        assert_eq!(
            listed,
            vec![
                ("hob".to_string(), cards),
                ("modules/riftbound".to_string(), plugin),
            ]
        );
        assert_eq!(
            list_under(&store, "modules"),
            vec![("modules/riftbound".to_string(), plugin)]
        );
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn traversal_and_hidden_names_are_refused() {
        let store = scratch("names");
        for name in [
            "../evil",
            "a/../b",
            "",
            ".hidden",
            "modules/.git",
            "a/b/c/d",
        ] {
            assert!(!valid_name(name), "{name} should be refused");
            assert!(path(&store, name).is_none());
            assert!(write(&store, name, BlobHash::of(b"x")).is_err());
        }
        assert!(valid_name("hob"));
        assert!(valid_name("modules/riftbound"));
        assert!(valid_name("cards/mtg"));
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_ref_holding_junk_is_skipped_not_fatal() {
        let store = scratch("junk");
        std::fs::create_dir_all(dir(&store)).unwrap();
        std::fs::write(dir(&store).join("broken"), "not a hash").unwrap();
        assert!(list(&store).is_empty());
        assert_eq!(read(&store, "broken"), None);
        let _ = std::fs::remove_dir_all(store.root());
    }
}
