use serde::de::{Deserialize, Deserializer, Error as DeError};
use serde::{Serialize, Serializer};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobHash([u8; 32]);

impl BlobHash {
    pub fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(hex: &str) -> Option<Self> {
        let hex = hex.trim();
        if hex.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(out))
    }
}

impl Serialize for BlobHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BlobHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| D::Error::custom(format!("{text:?} is not a blob hash")))
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Missing(BlobHash),
    Corrupt {
        expected: BlobHash,
        actual: BlobHash,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "store io: {e}"),
            StoreError::Missing(hash) => write!(f, "blob {hash} not in store"),
            StoreError::Corrupt { expected, actual } => {
                write!(f, "blob {expected} is corrupt (hashes to {actual})")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_of(&self, hash: BlobHash) -> PathBuf {
        self.root.join(hash.to_string())
    }

    fn blob_path(&self, hash: BlobHash) -> PathBuf {
        self.path_of(hash)
    }

    pub fn hashes(&self) -> Vec<BlobHash> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut found: Vec<BlobHash> = entries
            .flatten()
            .filter_map(|entry| BlobHash::parse(&entry.file_name().to_string_lossy()))
            .collect();
        found.sort();
        found
    }

    pub fn remove(&self, hash: BlobHash) -> Result<(), StoreError> {
        match std::fs::remove_file(self.blob_path(hash)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put(&self, bytes: &[u8]) -> Result<BlobHash, StoreError> {
        let hash = BlobHash::of(bytes);
        let path = self.blob_path(hash);
        if path.exists() {
            return Ok(hash);
        }
        let temp = self.root.join(format!("tmp-{hash}-{}", std::process::id()));
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &path)?;
        Ok(hash)
    }

    pub fn get(&self, hash: BlobHash) -> Result<Vec<u8>, StoreError> {
        let path = self.blob_path(hash);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::Missing(hash))
            }
            Err(e) => return Err(e.into()),
        };
        let actual = BlobHash::of(&bytes);
        if actual != hash {
            return Err(StoreError::Corrupt {
                expected: hash,
                actual,
            });
        }
        Ok(bytes)
    }

    pub fn has(&self, hash: BlobHash) -> bool {
        self.blob_path(hash).exists()
    }
}

pub trait Blobs {
    fn put(&self, bytes: &[u8]) -> Result<BlobHash, StoreError>;
    fn get(&self, hash: BlobHash) -> Result<Vec<u8>, StoreError>;
    fn has(&self, hash: BlobHash) -> bool;
}

impl Blobs for BlobStore {
    fn put(&self, bytes: &[u8]) -> Result<BlobHash, StoreError> {
        BlobStore::put(self, bytes)
    }

    fn get(&self, hash: BlobHash) -> Result<Vec<u8>, StoreError> {
        BlobStore::get(self, hash)
    }

    fn has(&self, hash: BlobHash) -> bool {
        BlobStore::has(self, hash)
    }
}

#[derive(Debug, Default)]
pub struct MemBlobs(std::sync::Mutex<std::collections::BTreeMap<BlobHash, Vec<u8>>>);

impl MemBlobs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Blobs for MemBlobs {
    fn put(&self, bytes: &[u8]) -> Result<BlobHash, StoreError> {
        let hash = BlobHash::of(bytes);
        self.0
            .lock()
            .unwrap()
            .entry(hash)
            .or_insert_with(|| bytes.to_vec());
        Ok(hash)
    }

    fn get(&self, hash: BlobHash) -> Result<Vec<u8>, StoreError> {
        self.0
            .lock()
            .unwrap()
            .get(&hash)
            .cloned()
            .ok_or(StoreError::Missing(hash))
    }

    fn has(&self, hash: BlobHash) -> bool {
        self.0.lock().unwrap().contains_key(&hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_store(tag: &str) -> BlobStore {
        let dir =
            std::env::temp_dir().join(format!("spirit-store-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    #[test]
    fn roundtrip() {
        let store = scratch_store("roundtrip");
        let hash = store.put(b"there and back again").unwrap();
        assert_eq!(store.get(hash).unwrap(), b"there and back again");
        assert!(store.has(hash));
    }

    #[test]
    fn put_is_idempotent_and_content_addressed() {
        let store = scratch_store("idempotent");
        let first = store.put(b"riddles in the dark").unwrap();
        let second = store.put(b"riddles in the dark").unwrap();
        assert_eq!(first, second);
        let other = store.put(b"an unexpected party").unwrap();
        assert_ne!(first, other);
    }

    #[test]
    fn missing_blob_is_an_error() {
        let store = scratch_store("missing");
        let hash = BlobHash::of(b"never stored");
        assert!(!store.has(hash));
        assert!(matches!(store.get(hash), Err(StoreError::Missing(_))));
    }

    #[test]
    fn corruption_is_detected_on_read() {
        let store = scratch_store("corrupt");
        let hash = store.put(b"the real contents").unwrap();
        std::fs::write(store.root().join(hash.to_string()), b"tampered").unwrap();
        assert!(matches!(store.get(hash), Err(StoreError::Corrupt { .. })));
    }

    #[test]
    fn hash_displays_and_parses_as_hex() {
        let hash = BlobHash::of(b"smaug");
        let hex = hash.to_string();
        assert_eq!(hex.len(), 64);
        assert_eq!(BlobHash::parse(&hex), Some(hash));
        assert_eq!(BlobHash::parse("zz"), None);
        assert_eq!(BlobHash::parse(&hex[..10]), None);
    }

    #[test]
    fn the_memory_store_behaves_like_the_file_store_through_the_trait() {
        fn exercise(store: &dyn Blobs) {
            let hash = store.put(b"leaves").unwrap();
            assert_eq!(store.put(b"leaves").unwrap(), hash);
            assert!(store.has(hash));
            assert_eq!(store.get(hash).unwrap(), b"leaves");
            assert!(matches!(
                store.get(BlobHash::of(b"absent")),
                Err(StoreError::Missing(_))
            ));
        }
        let mem = MemBlobs::new();
        exercise(&mem);
        assert_eq!(mem.len(), 1);
        let dir = std::env::temp_dir().join(format!("spirit-store-trait-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let disk = BlobStore::open(&dir).unwrap();
        exercise(&disk);
        assert_eq!(disk.hashes(), vec![BlobHash::of(b"leaves")]);
        disk.remove(BlobHash::of(b"leaves")).unwrap();
        assert!(disk.hashes().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
