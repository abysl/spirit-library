use serde::{Deserialize, Serialize};
use spirit_core::{canonical, refs, BlobHash, BlobStore};
use std::collections::BTreeMap;

pub const REF_NAME: &str = "assets";
pub const KIND: &str = "assets";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub kind: String,
    pub refs: Vec<BlobHash>,
    pub entries: BTreeMap<String, String>,
}

pub fn encode(entries: &BTreeMap<String, BlobHash>) -> Result<Vec<u8>, String> {
    let mut refs: Vec<BlobHash> = entries.values().copied().collect();
    refs.sort();
    refs.dedup();
    let index = Index {
        kind: KIND.to_string(),
        refs,
        entries: entries
            .iter()
            .map(|(key, hash)| (key.clone(), hash.to_string()))
            .collect(),
    };
    canonical::to_vec(&index).map_err(|error| error.to_string())
}

pub fn decode(bytes: &[u8]) -> Option<BTreeMap<String, BlobHash>> {
    let index: Index = canonical::from_slice(bytes).ok()?;
    if index.kind != KIND {
        return None;
    }
    Some(
        index
            .entries
            .into_iter()
            .filter_map(|(key, hash)| Some((key, BlobHash::parse(&hash)?)))
            .collect(),
    )
}

pub fn publish(
    store: &BlobStore,
    entries: &BTreeMap<String, BlobHash>,
) -> Result<BlobHash, String> {
    let bytes = encode(entries)?;
    let hash = store.put(&bytes).map_err(|error| error.to_string())?;
    if refs::read(store, REF_NAME) != Some(hash) {
        refs::write(store, REF_NAME, hash)?;
    }
    Ok(hash)
}

pub fn read(store: &BlobStore, manifest: BlobHash) -> Option<BTreeMap<String, BlobHash>> {
    decode(&store.get(manifest).ok()?)
}

pub fn local(store: &BlobStore) -> BTreeMap<String, BlobHash> {
    refs::read(store, REF_NAME)
        .and_then(|manifest| read(store, manifest))
        .unwrap_or_default()
}

pub fn lookup<'a>(
    indexes: impl IntoIterator<Item = (&'a str, &'a BTreeMap<String, BlobHash>)>,
    key: &str,
) -> Option<(BlobHash, String)> {
    let wanted = key.to_ascii_lowercase();
    indexes
        .into_iter()
        .find_map(|(peer, index)| index.get(&wanted).map(|hash| (*hash, peer.to_string())))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Held(BlobHash),
    Provider(BlobHash, String),
    Unknown,
}

pub fn find(
    store: &BlobStore,
    indexes: impl IntoIterator<Item = (String, BTreeMap<String, BlobHash>)>,
    key: &str,
) -> Found {
    let wanted = key.to_ascii_lowercase();
    let mut provider = None;
    for (peer, index) in indexes {
        if let Some(hash) = index.get(&wanted) {
            if store.has(*hash) {
                return Found::Held(*hash);
            }
            provider.get_or_insert((*hash, peer));
        }
    }
    match provider {
        Some((hash, peer)) => Found::Provider(hash, peer),
        None => Found::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("spirit-assets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_index_round_trips_and_declares_its_blobs_as_refs() {
        let dir = scratch("roundtrip");
        let store = BlobStore::open(&dir).unwrap();
        let mat = store.put(b"a playmat").unwrap();
        let back = store.put(b"a card back").unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("playmats/playmat:akali".to_string(), mat);
        entries.insert("card-backs/back:riftbound".to_string(), back);
        entries.insert("playmats/playmat:vi".to_string(), mat);
        let bytes = encode(&entries).unwrap();
        assert_eq!(decode(&bytes).unwrap(), entries);
        let envelope = spirit_core::envelope::of(&bytes).unwrap();
        assert_eq!(envelope.kind, KIND);
        assert_eq!(envelope.refs.len(), 2, "one ref per distinct blob");
        let manifest = publish(&store, &entries).unwrap();
        assert_eq!(refs::read(&store, REF_NAME), Some(manifest));
        assert_eq!(read(&store, manifest).unwrap(), entries);
        assert_eq!(local(&store), entries);
        assert_eq!(
            publish(&store, &entries).unwrap(),
            manifest,
            "republishing is idempotent"
        );
        assert!(decode(b"junk").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lookup_prefers_a_held_blob_and_names_a_provider_otherwise() {
        let dir = scratch("lookup");
        let store = BlobStore::open(&dir).unwrap();
        let held = store.put(b"held here").unwrap();
        let elsewhere = BlobHash::of(b"only on a peer");
        let mut theirs = BTreeMap::new();
        theirs.insert("playmats/playmat:akali".to_string(), elsewhere);
        theirs.insert("playmats/playmat:vi".to_string(), held);
        let mut ours = BTreeMap::new();
        ours.insert("playmats/playmat:akali".to_string(), elsewhere);
        let indexes = || {
            vec![
                ("peer-a".to_string(), theirs.clone()),
                ("peer-b".to_string(), ours.clone()),
            ]
        };
        assert_eq!(
            find(&store, indexes(), "playmats/playmat:Akali"),
            Found::Provider(elsewhere, "peer-a".into()),
            "keys are matched case-insensitively and the first provider wins"
        );
        assert_eq!(
            find(&store, indexes(), "playmats/playmat:vi"),
            Found::Held(held)
        );
        assert_eq!(
            find(&store, indexes(), "playmats/playmat:jinx"),
            Found::Unknown
        );
        assert_eq!(
            lookup(
                [("peer-a", &theirs), ("peer-b", &ours)],
                "PLAYMATS/playmat:akali"
            ),
            Some((elsewhere, "peer-a".into()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
