use crate::store::{BlobHash, BlobStore};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MODULE_REF_DIR: &str = "modules";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleKind {
    Engine,
    Plugin,
}

impl ModuleKind {
    pub fn label(&self) -> &'static str {
        match self {
            ModuleKind::Engine => "engine",
            ModuleKind::Plugin => "plugin",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleManifest {
    pub name: String,
    pub kind: ModuleKind,
    pub abi_version: u32,
    pub display: String,
    pub version: String,
    pub module: String,
}

impl ModuleManifest {
    pub fn module_hash(&self) -> Option<BlobHash> {
        BlobHash::parse(&self.module)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).expect("module manifest encodes");
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        ciborium::from_reader(bytes).ok()
    }
}

pub fn valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

pub fn module_ref_name(name: &str) -> String {
    format!("{MODULE_REF_DIR}/{name}")
}

pub fn strip_module_ref(ref_name: &str) -> Option<&str> {
    ref_name.strip_prefix(MODULE_REF_DIR)?.strip_prefix('/')
}

fn ref_path(store: &BlobStore, name: &str) -> PathBuf {
    store.root().join("refs").join(MODULE_REF_DIR).join(name)
}

pub fn publish_module(
    store: &BlobStore,
    manifest: &ModuleManifest,
    module: &[u8],
) -> Result<BlobHash, String> {
    if !valid_module_name(&manifest.name) {
        return Err(format!(
            "module name {:?} is not publishable",
            manifest.name
        ));
    }
    let module_hash = BlobHash::of(module);
    if manifest.module != module_hash.to_string() {
        return Err(format!(
            "manifest names module {} but the bytes hash to {module_hash}",
            manifest.module
        ));
    }
    store.put(module).map_err(|e| e.to_string())?;
    let manifest_hash = store.put(&manifest.encode()).map_err(|e| e.to_string())?;
    let path = ref_path(store, &manifest.name);
    let parent = path.parent().expect("module ref path has a parent");
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    std::fs::write(&path, format!("{manifest_hash}\n")).map_err(|e| e.to_string())?;
    Ok(manifest_hash)
}

pub fn module_manifest(store: &BlobStore, name: &str) -> Option<(ModuleManifest, BlobHash)> {
    let text = std::fs::read_to_string(ref_path(store, name)).ok()?;
    let manifest_hash = BlobHash::parse(&text)?;
    let manifest = ModuleManifest::decode(&store.get(manifest_hash).ok()?)?;
    Some((manifest, manifest_hash))
}

pub fn module_bytes(store: &BlobStore, name: &str) -> Result<(ModuleManifest, Vec<u8>), String> {
    let (manifest, _) =
        module_manifest(store, name).ok_or_else(|| format!("no module ref {name} in the store"))?;
    let hash = manifest
        .module_hash()
        .ok_or_else(|| format!("module ref {name} names an unparsable hash"))?;
    let bytes = store.get(hash).map_err(|e| e.to_string())?;
    Ok((manifest, bytes))
}

pub fn list_modules(store: &BlobStore) -> Vec<(ModuleManifest, BlobHash)> {
    let dir = store.root().join("refs").join(MODULE_REF_DIR);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| module_manifest(store, &name))
        .collect()
}

pub fn seed_module_if_absent(
    store: &BlobStore,
    manifest: &ModuleManifest,
    module: &[u8],
) -> Result<bool, String> {
    if let Some((existing, _)) = module_manifest(store, &manifest.name) {
        if existing.module_hash().is_some_and(|hash| store.has(hash)) {
            return Ok(false);
        }
    }
    publish_module(store, manifest, module)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_store(tag: &str) -> BlobStore {
        let dir =
            std::env::temp_dir().join(format!("spirit-modules-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    fn manifest_for(name: &str, kind: ModuleKind, module: &[u8]) -> ModuleManifest {
        ModuleManifest {
            name: name.into(),
            kind,
            abi_version: 0,
            display: format!("{name} module"),
            version: "0.1.0".into(),
            module: BlobHash::of(module).to_string(),
        }
    }

    #[test]
    fn a_manifest_survives_a_cbor_round_trip() {
        let manifest = manifest_for("riftbound", ModuleKind::Plugin, b"wasm bytes");
        let decoded = ModuleManifest::decode(&manifest.encode()).unwrap();
        assert_eq!(decoded, manifest);
        assert_eq!(decoded.kind.label(), "plugin");
        assert_eq!(decoded.module_hash(), Some(BlobHash::of(b"wasm bytes")));
    }

    #[test]
    fn garbage_bytes_do_not_decode_as_a_manifest() {
        assert!(ModuleManifest::decode(b"not cbor at all").is_none());
    }

    #[test]
    fn publish_stores_blob_manifest_and_ref() {
        let store = scratch_store("publish");
        let manifest = manifest_for("engine", ModuleKind::Engine, b"engine wasm");
        publish_module(&store, &manifest, b"engine wasm").unwrap();
        let (loaded, bytes) = module_bytes(&store, "engine").unwrap();
        assert_eq!(loaded, manifest);
        assert_eq!(bytes, b"engine wasm");
        let listed = list_modules(&store);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0.name, "engine");
    }

    #[test]
    fn publish_refuses_a_manifest_that_lies_about_its_module() {
        let store = scratch_store("lies");
        let mut manifest = manifest_for("engine", ModuleKind::Engine, b"real bytes");
        manifest.module = BlobHash::of(b"other bytes").to_string();
        assert!(publish_module(&store, &manifest, b"real bytes").is_err());
        assert!(module_manifest(&store, "engine").is_none());
    }

    #[test]
    fn publish_refuses_a_name_that_could_escape_the_refs_dir() {
        let store = scratch_store("escape");
        for name in ["../evil", "a/b", "", ".hidden"] {
            let manifest = manifest_for(name, ModuleKind::Plugin, b"bytes");
            assert!(publish_module(&store, &manifest, b"bytes").is_err());
        }
    }

    #[test]
    fn seeding_is_idempotent_and_recovers_a_deleted_store() {
        let store = scratch_store("seed");
        let manifest = manifest_for("engine", ModuleKind::Engine, b"engine wasm");
        assert!(seed_module_if_absent(&store, &manifest, b"engine wasm").unwrap());
        assert!(!seed_module_if_absent(&store, &manifest, b"engine wasm").unwrap());
        let root = store.root().to_path_buf();
        drop(store);
        std::fs::remove_dir_all(&root).unwrap();
        let store = BlobStore::open(&root).unwrap();
        assert!(seed_module_if_absent(&store, &manifest, b"engine wasm").unwrap());
        assert_eq!(module_bytes(&store, "engine").unwrap().1, b"engine wasm");
    }

    #[test]
    fn seeding_never_clobbers_an_installed_module() {
        let store = scratch_store("installed");
        let installed = manifest_for("engine", ModuleKind::Engine, b"mesh engine");
        publish_module(&store, &installed, b"mesh engine").unwrap();
        let bundled = manifest_for("engine", ModuleKind::Engine, b"bundled engine");
        assert!(!seed_module_if_absent(&store, &bundled, b"bundled engine").unwrap());
        assert_eq!(module_bytes(&store, "engine").unwrap().1, b"mesh engine");
    }

    #[test]
    fn ref_names_round_trip_through_the_modules_prefix() {
        assert_eq!(module_ref_name("engine"), "modules/engine");
        assert_eq!(strip_module_ref("modules/engine"), Some("engine"));
        assert_eq!(strip_module_ref("hob"), None);
        assert_eq!(strip_module_ref("modulesengine"), None);
    }
}
