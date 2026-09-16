use serde::{Deserialize, Serialize};
use spirit_core::canonical::{self, CanonError};
use spirit_core::collection::{self, Item};
use spirit_core::record::Cir;
use spirit_core::{identity, BlobHash, BlobStore, CiHash, Dgid, Identity, Signature};

pub const DEVICE_KIND: &str = "device";
pub const GROUP_COLLECTION: &str = "device-group";
pub const NODE_PREFIX: &str = "nodeid:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub dgid: Dgid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Settings {
    pub fn for_group(dgid: Dgid) -> Self {
        Self {
            dgid,
            expires: None,
            role: None,
            tags: Vec::new(),
        }
    }

    pub fn scope(&self) -> Result<Vec<u8>, CanonError> {
        Ok(canonical::hash(self)?.as_bytes().to_vec())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub device_sig: Signature,
    pub pubkey: String,
    pub settings: Settings,
}

impl Device {
    pub fn sign(device: &Identity, settings: Settings) -> Result<Self, CanonError> {
        let device_sig = device.sign(&settings.scope()?);
        Ok(Self {
            device_sig,
            pubkey: format!("{NODE_PREFIX}{}", hex(device.dgid().as_bytes())),
            settings,
        })
    }

    pub fn node_id(&self) -> Option<[u8; 32]> {
        let text = self
            .pubkey
            .strip_prefix(NODE_PREFIX)
            .unwrap_or(&self.pubkey);
        Dgid::parse(text).map(|dgid| *dgid.as_bytes())
    }

    pub fn node_hex(&self) -> String {
        self.pubkey
            .strip_prefix(NODE_PREFIX)
            .unwrap_or(&self.pubkey)
            .to_string()
    }

    pub fn verify(&self) -> bool {
        let Some(node_id) = self.node_id() else {
            return false;
        };
        let Ok(scope) = self.settings.scope() else {
            return false;
        };
        identity::verify(Dgid::from_bytes(node_id), &scope, self.device_sig)
    }

    pub fn cir(&self) -> Result<Cir, CanonError> {
        Cir::new(DEVICE_KIND, self)
    }

    pub fn ci(&self) -> Result<CiHash, CanonError> {
        self.cir()?.address()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub ci: CiHash,
    pub device: Device,
    pub verified: bool,
    pub expired: bool,
}

impl Member {
    pub fn current(&self) -> bool {
        self.verified && !self.expired
    }
}

pub use spirit_core::clock::{expired, now_rfc3339};

pub fn members(store: &BlobStore, group: Dgid) -> Vec<Member> {
    let Some((head, ops)) = collection::load(store, GROUP_COLLECTION) else {
        return Vec::new();
    };
    let owned = head.owner == group;
    let now = now_rfc3339();
    collection::fold(head.owner, &ops)
        .into_iter()
        .filter_map(|item| {
            let cir = Cir::decode(&store.get(item.ci.hash()).ok()?).ok()?;
            if cir.kind != DEVICE_KIND {
                return None;
            }
            let device: Device = cir.body().ok()?;
            let verified = owned && device.verify() && device.settings.dgid == group;
            let expired = expired(device.settings.expires.as_deref(), &now);
            Some(Member {
                ci: item.ci,
                device,
                verified,
                expired,
            })
        })
        .collect()
}

pub fn member_ids(store: &BlobStore, group: Dgid) -> Vec<String> {
    members(store, group)
        .into_iter()
        .filter(Member::current)
        .map(|member| member.device.node_hex())
        .collect()
}

pub struct Admitted {
    pub ci: CiHash,
    pub head: BlobHash,
    pub already: bool,
}

pub fn admit(store: &BlobStore, group: &Identity, device: &Device) -> Result<Admitted, String> {
    if !device.verify() {
        return Err("the device record's signature does not verify".into());
    }
    if device.settings.dgid != group.dgid() {
        return Err(format!(
            "the device offers itself to {} not to this group",
            device.settings.dgid
        ));
    }
    let cir = device.cir().map_err(|e| e.to_string())?;
    let ci = store
        .put(&cir.encode().map_err(|e| e.to_string())?)
        .map(CiHash::from_hash)
        .map_err(|e| e.to_string())?;
    let already = members(store, group.dgid())
        .iter()
        .any(|member| member.ci == ci && member.current());
    if already {
        let head = spirit_core::refs::read(store, GROUP_COLLECTION).ok_or("no group head")?;
        return Ok(Admitted {
            ci,
            head,
            already: true,
        });
    }
    let mut builder = collection::Builder::open(store, GROUP_COLLECTION, group);
    builder.kind(GROUP_COLLECTION);
    for stale in members(store, group.dgid())
        .into_iter()
        .filter(|member| member.device.pubkey == device.pubkey)
    {
        builder.remove(stale.ci);
    }
    builder
        .add(Item::labelled(ci, device.node_hex()))
        .record(ci.hash());
    let head = builder.publish(store, group)?;
    Ok(Admitted {
        ci,
        head,
        already: false,
    })
}

pub fn revoke(
    store: &BlobStore,
    group: &Identity,
    node_hex: &str,
) -> Result<Option<BlobHash>, String> {
    let targets: Vec<CiHash> = members(store, group.dgid())
        .into_iter()
        .filter(|member| member.device.node_hex() == node_hex)
        .map(|member| member.ci)
        .collect();
    if targets.is_empty() {
        return Ok(None);
    }
    let mut builder = collection::Builder::open(store, GROUP_COLLECTION, group);
    builder.kind(GROUP_COLLECTION);
    for ci in targets {
        builder.remove(ci);
    }
    builder.publish(store, group).map(Some)
}

pub fn ensure_self(
    store: &BlobStore,
    group: &Identity,
    device: &Identity,
) -> Result<Admitted, String> {
    let record =
        Device::sign(device, Settings::for_group(group.dgid())).map_err(|e| e.to_string())?;
    admit(store, group, &record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> BlobStore {
        let dir = std::env::temp_dir().join(format!("spirit-device-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    #[test]
    fn a_device_record_carries_the_devices_own_consent() {
        let group = Identity::from_secret([1; 32]);
        let device = Identity::from_secret([2; 32]);
        let record = Device::sign(&device, Settings::for_group(group.dgid())).unwrap();
        assert!(record.verify());
        assert_eq!(record.node_id(), Some(*device.dgid().as_bytes()));
        assert!(record.pubkey.starts_with("nodeid:"));
        let mut tampered = record.clone();
        tampered.settings.tags.push("cdn".into());
        assert!(!tampered.verify());
        let cir = record.cir().unwrap();
        assert_eq!(cir.kind, DEVICE_KIND);
        let back: Device = Cir::decode(&cir.encode().unwrap()).unwrap().body().unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn a_group_of_one_admits_itself_then_a_second_device() {
        let store = scratch("admit");
        let group = Identity::from_secret([1; 32]);
        let me = Identity::from_secret([2; 32]);
        let other = Identity::from_secret([3; 32]);
        let first = ensure_self(&store, &group, &me).unwrap();
        assert!(!first.already);
        let again = ensure_self(&store, &group, &me).unwrap();
        assert!(again.already);
        assert_eq!(again.head, first.head);
        let record = Device::sign(&other, Settings::for_group(group.dgid())).unwrap();
        let admitted = admit(&store, &group, &record).unwrap();
        assert!(!admitted.already);
        let ids = member_ids(&store, group.dgid());
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&record.node_hex()));
        let (head, _) = collection::load(&store, GROUP_COLLECTION).unwrap();
        assert_eq!(head.kind, GROUP_COLLECTION);
        assert_eq!(head.owner, group.dgid());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_device_offered_to_another_group_or_expired_is_not_a_member() {
        let store = scratch("foreign");
        let group = Identity::from_secret([1; 32]);
        let stranger_group = Identity::from_secret([9; 32]);
        let device = Identity::from_secret([2; 32]);
        let foreign = Device::sign(&device, Settings::for_group(stranger_group.dgid())).unwrap();
        assert!(admit(&store, &group, &foreign).is_err());
        let mut settings = Settings::for_group(group.dgid());
        settings.expires = Some("2000-01-01T00:00:00Z".into());
        let stale = Device::sign(&device, settings).unwrap();
        admit(&store, &group, &stale).unwrap();
        let listed = members(&store, group.dgid());
        assert_eq!(listed.len(), 1);
        assert!(listed[0].expired);
        assert!(member_ids(&store, group.dgid()).is_empty());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn joining_a_new_group_sheds_the_record_from_the_old_one() {
        let store = scratch("shed");
        let old_group = Identity::from_secret([1; 32]);
        let new_group = Identity::from_secret([4; 32]);
        let me = Identity::from_secret([2; 32]);
        ensure_self(&store, &old_group, &me).unwrap();
        assert!(members(&store, new_group.dgid())
            .iter()
            .all(|m| !m.verified));
        ensure_self(&store, &new_group, &me).unwrap();
        let listed = members(&store, new_group.dgid());
        assert_eq!(listed.len(), 1);
        assert!(listed[0].verified);
        assert_eq!(listed[0].device.settings.dgid, new_group.dgid());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn revoking_drops_the_device_and_re_admitting_restores_it() {
        let store = scratch("revoke");
        let group = Identity::from_secret([1; 32]);
        let device = Identity::from_secret([2; 32]);
        let record = Device::sign(&device, Settings::for_group(group.dgid())).unwrap();
        admit(&store, &group, &record).unwrap();
        assert!(revoke(&store, &group, &record.node_hex())
            .unwrap()
            .is_some());
        assert!(member_ids(&store, group.dgid()).is_empty());
        assert!(revoke(&store, &group, &record.node_hex())
            .unwrap()
            .is_none());
        admit(&store, &group, &record).unwrap();
        assert_eq!(member_ids(&store, group.dgid()).len(), 1);
        let _ = std::fs::remove_dir_all(store.root());
    }
}
