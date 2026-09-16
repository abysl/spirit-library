use serde::{Deserialize, Serialize};
use spirit_core::canonical::CanonError;
use spirit_core::collection::{self, Item};
use spirit_core::record::{Attestation, Cir, Claim, Tdr};
use spirit_core::{
    refs, BlobHash, BlobRef, BlobStore, CiHash, Dgid, Identity, TdHash, Trust, TrustLevel,
};

pub const MODULE_KIND: &str = "wasm-module";
pub const MODULE_PREFIX: &str = "modules";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Engine,
    Plugin,
}

impl Role {
    pub fn label(&self) -> &'static str {
        match self {
            Role::Engine => "engine",
            Role::Plugin => "plugin",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub abi_version: u32,
    pub name: String,
    pub role: Role,
    pub version: String,
}

impl Module {
    pub fn new(name: &str, role: Role, version: &str, abi_version: u32) -> Self {
        Self {
            abi_version,
            name: name.into(),
            role,
            version: version.into(),
        }
    }

    pub fn cir(&self) -> Result<Cir, CanonError> {
        Cir::new(MODULE_KIND, self)
    }

    pub fn ci(&self) -> Result<CiHash, CanonError> {
        self.cir()?.address()
    }
}

pub fn collection_name(name: &str) -> String {
    format!("{MODULE_PREFIX}/{name}")
}

pub fn module_name(collection: &str) -> Option<&str> {
    collection
        .strip_prefix(MODULE_PREFIX)?
        .strip_prefix('/')
        .filter(|name| refs::valid_segment(name))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub ci: CiHash,
    pub module: Module,
    pub blob: Option<BlobHash>,
    pub td: Option<TdHash>,
    pub signer: Option<Dgid>,
    pub owner: Option<Dgid>,
    pub held: bool,
    pub legacy: bool,
}

impl Version {
    pub fn trusted(&self, trust: &Trust, at_least: TrustLevel) -> bool {
        match self.signer {
            Some(dgid) => trust.trusts(dgid, at_least),
            None => self.legacy,
        }
    }
}

pub struct Published {
    pub ci: CiHash,
    pub blob: BlobHash,
    pub head: BlobHash,
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn newer(left: &Version, right: &Version) -> bool {
    let (mine, theirs) = (
        version_key(&left.module.version),
        version_key(&right.module.version),
    );
    match mine.cmp(&theirs) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => left.module.version > right.module.version,
    }
}

fn read_cir(store: &BlobStore, ci: CiHash) -> Option<Module> {
    let cir = Cir::decode(&store.get(ci.hash()).ok()?).ok()?;
    (cir.kind == MODULE_KIND).then_some(())?;
    cir.body().ok()
}

fn attestation_for(
    store: &BlobStore,
    attestations: &[BlobHash],
    ci: CiHash,
) -> Option<Attestation> {
    let claims: Vec<Attestation> = attestations
        .iter()
        .filter_map(|hash| Attestation::decode(&store.get(*hash).ok()?).ok())
        .filter(|attestation| attestation.claim.ci == ci)
        .collect();
    claims
        .iter()
        .rfind(|attestation| {
            attestation
                .claim
                .blob
                .is_some_and(|blob| store.has(blob.hash()))
        })
        .or_else(|| claims.last())
        .cloned()
}

pub fn versions(store: &BlobStore, name: &str) -> Vec<Version> {
    let collection = collection_name(name);
    let Some((head, ops)) = collection::load(store, &collection) else {
        return legacy_versions(store, name).into_iter().collect();
    };
    collection::fold(head.owner, &ops)
        .into_iter()
        .filter_map(|item| {
            let module = read_cir(store, item.ci)?;
            let attestation = attestation_for(store, &head.attestations, item.ci);
            let blob = attestation
                .as_ref()
                .and_then(|found| found.claim.blob)
                .map(|blob| blob.hash());
            Some(Version {
                ci: item.ci,
                module,
                blob,
                td: attestation.as_ref().and_then(|found| found.claim.td),
                signer: attestation.as_ref().and_then(|found| found.signer()),
                owner: Some(head.owner),
                held: blob.is_some_and(|hash| store.has(hash)),
                legacy: false,
            })
        })
        .collect()
}

fn legacy_versions(store: &BlobStore, name: &str) -> Option<Version> {
    let (manifest, _) = spirit_core::modules::module_manifest(store, name)?;
    let blob = manifest.module_hash();
    let module = Module::new(
        &manifest.name,
        match manifest.kind {
            spirit_core::modules::ModuleKind::Engine => Role::Engine,
            spirit_core::modules::ModuleKind::Plugin => Role::Plugin,
        },
        &manifest.version,
        manifest.abi_version,
    );
    Some(Version {
        ci: module.ci().ok()?,
        module,
        blob,
        td: None,
        signer: None,
        owner: None,
        held: blob.is_some_and(|hash| store.has(hash)),
        legacy: true,
    })
}

pub fn resolve(
    store: &BlobStore,
    trust: &Trust,
    name: &str,
    abi_version: Option<u32>,
) -> Option<Version> {
    versions(store, name)
        .into_iter()
        .filter(|version| version.held && version.trusted(trust, TrustLevel::Cache))
        .filter(|version| abi_version.is_none_or(|abi| version.module.abi_version == abi))
        .reduce(|best, version| {
            if newer(&version, &best) {
                version
            } else {
                best
            }
        })
}

pub fn bytes(store: &BlobStore, version: &Version) -> Result<Vec<u8>, String> {
    let blob = version
        .blob
        .ok_or_else(|| format!("module {} names no bytes", version.module.name))?;
    store.get(blob).map_err(|e| e.to_string())
}

pub fn publish(
    store: &BlobStore,
    identity: &Identity,
    module: &Module,
    td: &Tdr,
    wasm: &[u8],
) -> Result<Published, String> {
    if !refs::valid_segment(&module.name) {
        return Err(format!("module name {:?} is not publishable", module.name));
    }
    let collection = collection_name(&module.name);
    let blob = store.put(wasm).map_err(|e| e.to_string())?;
    let cir = module.cir().map_err(|e| e.to_string())?;
    let ci = store
        .put(&cir.encode().map_err(|e| e.to_string())?)
        .map(CiHash::from_hash)
        .map_err(|e| e.to_string())?;
    let td_hash = store
        .put(&td.encode().map_err(|e| e.to_string())?)
        .map(TdHash::from_hash)
        .map_err(|e| e.to_string())?;
    let attestation = Attestation::sign(
        Claim::content(ci, td_hash, BlobRef::from_hash(blob)),
        identity,
    )
    .map_err(|e| e.to_string())?;
    let attestation_hash = store
        .put(&attestation.encode().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut builder = collection::Builder::open(store, &collection, identity);
    builder
        .kind(MODULE_PREFIX)
        .add(Item::labelled(ci, module.version.clone()))
        .attest(attestation_hash)
        .record(ci.hash())
        .record(td_hash.hash())
        .record(blob);
    let head_hash = builder.publish(store, identity)?;
    Ok(Published {
        ci,
        blob,
        head: head_hash,
    })
}

pub fn list(store: &BlobStore) -> Vec<(String, Vec<Version>)> {
    let mut names: Vec<String> = refs::list_under(store, MODULE_PREFIX)
        .into_iter()
        .filter_map(|(name, _)| module_name(&name).map(String::from))
        .collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let found = versions(store, &name);
            (name, found)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> BlobStore {
        let dir = std::env::temp_dir().join(format!("spirit-modules-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    #[derive(Serialize)]
    struct HardenConfig {
        gas_limit: u64,
    }

    fn td() -> Tdr {
        Tdr::new("wasm-harden", &HardenConfig { gas_limit: 1000 }).unwrap()
    }

    fn publish_at(
        store: &BlobStore,
        identity: &Identity,
        name: &str,
        version: &str,
        wasm: &[u8],
    ) -> Published {
        publish(
            store,
            identity,
            &Module::new(name, Role::Plugin, version, 3),
            &td(),
            wasm,
        )
        .unwrap()
    }

    fn trust_of(identity: &Identity) -> Trust {
        Trust::new().with_own(identity.dgid())
    }

    #[test]
    fn resolving_a_version_never_reads_the_wasm_blob_to_find_its_claim() {
        let store = scratch("cheap");
        let me = Identity::from_secret([12; 32]);
        publish_at(&store, &me, "engine", "0.1.0", b"pretend this is megabytes");
        let (head, _) = collection::load(&store, "modules/engine").unwrap();
        assert_eq!(head.attestations.len(), 1);
        let blob = BlobHash::of(b"pretend this is megabytes");
        assert!(!head.attestations.contains(&blob));
        assert!(head.records.contains(&blob));
        assert!(head.refs.contains(&blob));
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_published_module_resolves_back_to_its_bytes() {
        let store = scratch("publish");
        let me = Identity::from_secret([1; 32]);
        let published = publish_at(&store, &me, "riftbound", "0.4.0", b"plugin bytes");
        let trust = trust_of(&me);
        let resolved = resolve(&store, &trust, "riftbound", Some(3)).unwrap();
        assert_eq!(resolved.ci, published.ci);
        assert_eq!(resolved.module.version, "0.4.0");
        assert_eq!(resolved.signer, Some(me.dgid()));
        assert!(resolved.held);
        assert_eq!(bytes(&store, &resolved).unwrap(), b"plugin bytes");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn versions_accumulate_and_the_newest_one_resolves() {
        let store = scratch("versions");
        let me = Identity::from_secret([2; 32]);
        publish_at(&store, &me, "riftbound", "0.3.1", b"v3");
        publish_at(&store, &me, "riftbound", "0.10.0", b"v10");
        publish_at(&store, &me, "riftbound", "0.4.0", b"v4");
        let trust = trust_of(&me);

        let all = versions(&store, "riftbound");
        assert_eq!(all.len(), 3);
        let resolved = resolve(&store, &trust, "riftbound", None).unwrap();
        assert_eq!(resolved.module.version, "0.10.0");
        assert_eq!(bytes(&store, &resolved).unwrap(), b"v10");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn republishing_the_same_version_does_not_duplicate_it() {
        let store = scratch("idempotent");
        let me = Identity::from_secret([3; 32]);
        publish_at(&store, &me, "mtg", "1.0.0", b"same bytes");
        publish_at(&store, &me, "mtg", "1.0.0", b"same bytes");
        assert_eq!(versions(&store, "mtg").len(), 1);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_bundle_publish_carries_a_mesh_install_forward_instead_of_clobbering_it() {
        let store = scratch("fork");
        let peer = Identity::from_secret([4; 32]);
        let me = Identity::from_secret([5; 32]);
        publish_at(&store, &peer, "riftbound", "0.9.0", b"mesh bytes");
        publish_at(&store, &me, "riftbound", "0.4.0", b"bundled bytes");

        let all = versions(&store, "riftbound");
        assert_eq!(all.len(), 2);
        let mut trust = Trust::new().with_own(me.dgid());
        trust.set(peer.dgid(), TrustLevel::Cache);
        let resolved = resolve(&store, &trust, "riftbound", None).unwrap();
        assert_eq!(resolved.module.version, "0.9.0");
        assert_eq!(resolved.signer, Some(peer.dgid()));
        assert_eq!(bytes(&store, &resolved).unwrap(), b"mesh bytes");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn republishing_a_version_with_new_bytes_resolves_to_the_new_bytes() {
        let store = scratch("rebuild");
        let me = Identity::from_secret([11; 32]);
        publish_at(&store, &me, "engine", "0.1.0", b"first build");
        publish_at(&store, &me, "engine", "0.1.0", b"second build");
        let trust = trust_of(&me);
        let resolved = resolve(&store, &trust, "engine", None).unwrap();
        assert_eq!(bytes(&store, &resolved).unwrap(), b"second build");
        assert_eq!(versions(&store, "engine").len(), 1);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn an_untrusted_signers_version_never_resolves() {
        let store = scratch("untrusted");
        let stranger = Identity::from_secret([6; 32]);
        let me = Identity::from_secret([7; 32]);
        publish_at(&store, &stranger, "riftbound", "9.9.9", b"hostile bytes");
        publish_at(&store, &me, "riftbound", "0.1.0", b"my bytes");

        let trust = Trust::new().with_own(me.dgid());
        let resolved = resolve(&store, &trust, "riftbound", None).unwrap();
        assert_eq!(resolved.module.version, "0.1.0");
        assert_eq!(bytes(&store, &resolved).unwrap(), b"my bytes");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn an_abi_mismatch_is_not_resolvable() {
        let store = scratch("abi");
        let me = Identity::from_secret([8; 32]);
        publish_at(&store, &me, "mtg", "1.0.0", b"bytes");
        let trust = trust_of(&me);
        assert!(resolve(&store, &trust, "mtg", Some(3)).is_some());
        assert!(resolve(&store, &trust, "mtg", Some(4)).is_none());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_legacy_module_ref_still_resolves_and_then_upgrades() {
        let store = scratch("legacy");
        let me = Identity::from_secret([9; 32]);
        spirit_core::modules::publish_module(
            &store,
            &spirit_core::modules::ModuleManifest {
                name: "engine".into(),
                kind: spirit_core::modules::ModuleKind::Engine,
                abi_version: 3,
                display: "agni engine".into(),
                version: "0.2.0".into(),
                module: BlobHash::of(b"legacy engine").to_string(),
            },
            b"legacy engine",
        )
        .unwrap();

        let trust = trust_of(&me);
        let resolved = resolve(&store, &trust, "engine", Some(3)).unwrap();
        assert!(resolved.legacy);
        assert_eq!(resolved.module.role, Role::Engine);
        assert_eq!(bytes(&store, &resolved).unwrap(), b"legacy engine");

        publish(
            &store,
            &me,
            &Module::new("engine", Role::Engine, "0.3.0", 3),
            &td(),
            b"new engine",
        )
        .unwrap();
        let resolved = resolve(&store, &trust, "engine", Some(3)).unwrap();
        assert!(!resolved.legacy);
        assert_eq!(bytes(&store, &resolved).unwrap(), b"new engine");
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn listing_names_every_module_with_its_versions() {
        let store = scratch("list");
        let me = Identity::from_secret([10; 32]);
        publish_at(&store, &me, "riftbound", "0.1.0", b"a");
        publish_at(&store, &me, "riftbound", "0.2.0", b"b");
        publish_at(&store, &me, "mtg", "0.1.0", b"c");
        let listed = list(&store);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "mtg");
        assert_eq!(listed[0].1.len(), 1);
        assert_eq!(listed[1].0, "riftbound");
        assert_eq!(listed[1].1.len(), 2);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn collection_names_round_trip() {
        assert_eq!(collection_name("riftbound"), "modules/riftbound");
        assert_eq!(module_name("modules/riftbound"), Some("riftbound"));
        assert_eq!(module_name("cards/mtg"), None);
        assert_eq!(module_name("modules"), None);
    }
}
