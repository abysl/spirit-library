use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dgid([u8; 32]);

impl Dgid {
    pub const PREFIX: &'static str = "dgid:";

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.trim();
        let hex = hex.strip_prefix(Self::PREFIX).unwrap_or(hex);
        Some(Self(parse_hex::<32>(hex)?))
    }

    pub fn short(&self) -> String {
        self.0[..4].iter().map(|b| format!("{b:02x}")).collect()
    }

    fn verifying_key(&self) -> Option<VerifyingKey> {
        VerifyingKey::from_bytes(&self.0).ok()
    }
}

impl fmt::Display for Dgid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Self::PREFIX)?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Dgid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Dgid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not a dgid")))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; 64]);

impl Signature {
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(Self(parse_hex::<64>(text.trim())?))
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Signature {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| serde::de::Error::custom("signature is not 64 hex-encoded bytes"))
    }
}

fn parse_hex<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub struct Identity(SigningKey);

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("dgid", &self.dgid())
            .finish()
    }
}

impl Identity {
    pub fn from_secret(bytes: [u8; 32]) -> Self {
        Self(SigningKey::from_bytes(&bytes))
    }

    pub fn generate() -> Result<Self, std::io::Error> {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(std::io::Error::other)?;
        Ok(Self::from_secret(secret))
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn dgid(&self) -> Dgid {
        Dgid(self.0.verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message).to_bytes())
    }
}

pub fn verify(dgid: Dgid, message: &[u8], signature: Signature) -> bool {
    let Some(key) = dgid.verifying_key() else {
        return false;
    };
    key.verify(message, &ed25519_dalek::Signature::from_bytes(&signature.0))
        .is_ok()
}

pub fn key_path(dir: &Path) -> PathBuf {
    dir.join("identity").join("key")
}

pub fn device_key_path(dir: &Path) -> PathBuf {
    dir.join("identity").join("node")
}

pub fn load(dir: &Path) -> Option<Identity> {
    read(dir).ok().flatten()
}

pub fn read(dir: &Path) -> std::io::Result<Option<Identity>> {
    read_at(&key_path(dir))
}

pub fn load_or_create(dir: &Path) -> std::io::Result<Identity> {
    load_or_create_at(&key_path(dir))
}

pub fn store(dir: &Path, identity: &Identity) -> std::io::Result<()> {
    store_at(&key_path(dir), identity)
}

pub fn load_device(dir: &Path) -> Option<Identity> {
    read_at(&device_key_path(dir)).ok().flatten()
}

pub fn load_or_create_device(dir: &Path) -> std::io::Result<Identity> {
    load_or_create_at(&device_key_path(dir))
}

pub fn store_device(dir: &Path, identity: &Identity) -> std::io::Result<()> {
    store_at(&device_key_path(dir), identity)
}

fn read_at(path: &Path) -> std::io::Result<Option<Identity>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    parse_hex::<32>(text.trim())
        .map(Identity::from_secret)
        .map(Some)
        .ok_or_else(|| std::io::Error::other(format!("bad identity key at {}", path.display())))
}

fn load_or_create_at(path: &Path) -> std::io::Result<Identity> {
    if let Some(identity) = read_at(path)? {
        return Ok(identity);
    }
    let identity = Identity::generate()?;
    store_at(path, &identity)?;
    Ok(identity)
}

fn store_at(path: &Path, identity: &Identity) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("identity path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let hex: String = identity
        .secret_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    std::fs::write(path, format!("{hex}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn vouch(group: &Identity, node_id: &[u8; 32]) -> Signature {
    group.sign(node_id)
}

pub fn vouched(dgid: Dgid, node_id: &[u8; 32], signature: Signature) -> bool {
    verify(dgid, node_id, signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("spirit-identity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn identity(seed: u8) -> Identity {
        Identity::from_secret([seed; 32])
    }

    #[test]
    fn a_signature_verifies_against_its_dgid_and_nothing_else() {
        let mine = identity(7);
        let theirs = identity(9);
        let claim = b"the module is the bytes";
        let signature = mine.sign(claim);
        assert!(verify(mine.dgid(), claim, signature));
        assert!(!verify(theirs.dgid(), claim, signature));
        assert!(!verify(mine.dgid(), b"different claim", signature));
    }

    #[test]
    fn a_dgid_round_trips_through_its_text_form() {
        let dgid = identity(3).dgid();
        let text = dgid.to_string();
        assert!(text.starts_with("dgid:"));
        assert_eq!(Dgid::parse(&text), Some(dgid));
        assert_eq!(Dgid::parse(&text[5..]), Some(dgid));
        assert_eq!(Dgid::parse("dgid:zz"), None);
        assert_eq!(dgid.short().len(), 8);
    }

    #[test]
    fn a_signature_round_trips_through_canonical_cbor() {
        let signature = identity(5).sign(b"claim");
        let bytes = crate::canonical::to_vec(&signature).unwrap();
        assert_eq!(
            crate::canonical::from_slice::<Signature>(&bytes).unwrap(),
            signature
        );
    }

    #[test]
    fn load_or_create_mints_once_and_then_reloads() {
        let dir = scratch("create");
        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first.dgid(), second.dgid());
        assert_ne!(first.dgid(), Identity::generate().unwrap().dgid());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_key_is_an_error_and_is_never_replaced() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(key_path(&dir).parent().unwrap()).unwrap();
        std::fs::write(key_path(&dir), "not a key").unwrap();
        assert!(read(&dir).is_err());
        assert!(load_or_create(&dir).is_err());
        assert_eq!(
            std::fs::read_to_string(key_path(&dir)).unwrap(),
            "not a key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_key_loads_back_as_the_same_dgid() {
        let dir = scratch("store");
        assert!(load(&dir).is_none());
        let mine = identity(11);
        store(&dir, &mine).unwrap();
        assert_eq!(load(&dir).unwrap().dgid(), mine.dgid());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_device_key_and_the_group_key_are_separate_files() {
        let dir = scratch("device");
        let group = load_or_create(&dir).unwrap();
        let device = load_or_create_device(&dir).unwrap();
        assert_ne!(group.dgid(), device.dgid());
        assert_eq!(load_or_create(&dir).unwrap().dgid(), group.dgid());
        assert_eq!(load_or_create_device(&dir).unwrap().dgid(), device.dgid());
        assert!(key_path(&dir).ends_with("identity/key"));
        assert!(device_key_path(&dir).ends_with("identity/node"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vouch_binds_one_node_id_to_one_group() {
        let group = identity(4);
        let other = identity(5);
        let node = [7u8; 32];
        let signature = vouch(&group, &node);
        assert!(vouched(group.dgid(), &node, signature));
        assert!(!vouched(other.dgid(), &node, signature));
        assert!(!vouched(group.dgid(), &[8u8; 32], signature));
    }
}
