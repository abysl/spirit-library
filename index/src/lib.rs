use ciborium::value::Value;
use spirit_core::collection::{self, Collection};
use spirit_core::record::{Attestation, Cir};
use spirit_core::{BlobHash, BlobStore, CiHash, Dgid, Trust, TrustLevel};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Index {
    kinds: BTreeMap<CiHash, String>,
    attestations: BTreeMap<CiHash, Vec<Attestation>>,
    links: BTreeMap<CiHash, Vec<CiHash>>,
    external: BTreeMap<(String, String), CiHash>,
    collections: BTreeMap<String, Dgid>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(store: &BlobStore) -> Self {
        let mut index = Self::new();
        for (name, _) in spirit_core::refs::list(store) {
            index.absorb_ref(store, &name);
        }
        index
    }

    pub fn absorb_ref(&mut self, store: &BlobStore, name: &str) {
        let Some((head, _)) = collection::load(store, name) else {
            self.absorb_legacy(store, name);
            return;
        };
        self.collections.insert(head.name.clone(), head.owner);
        self.absorb_records(store, &head);
    }

    fn absorb_legacy(&mut self, store: &BlobStore, name: &str) {
        let Some(hash) = spirit_core::refs::read(store, name) else {
            return;
        };
        let Ok(bytes) = store.get(hash) else {
            return;
        };
        let Some(envelope) = spirit_core::envelope::of(&bytes) else {
            return;
        };
        for hash in envelope.refs {
            self.absorb_blob(store, hash);
        }
    }

    fn absorb_records(&mut self, store: &BlobStore, head: &Collection) {
        for hash in &head.records {
            self.absorb_blob(store, *hash);
        }
    }

    fn absorb_blob(&mut self, store: &BlobStore, hash: BlobHash) {
        let Ok(bytes) = store.get(hash) else {
            return;
        };
        if let Ok(attestation) = Attestation::decode(&bytes) {
            let claims = self.attestations.entry(attestation.claim.ci).or_default();
            if !claims.contains(&attestation) {
                claims.push(attestation);
            }
            return;
        }
        if let Ok(cir) = Cir::decode(&bytes) {
            let ci = CiHash::from_hash(hash);
            self.kinds.insert(ci, cir.kind.clone());
            self.absorb_body(ci, &cir.body);
        }
    }

    fn absorb_body(&mut self, ci: CiHash, body: &Value) {
        let Value::Map(entries) = body else {
            return;
        };
        for (key, value) in entries {
            let Value::Text(key) = key else {
                continue;
            };
            match value {
                Value::Text(text) => {
                    if let Some(target) = CiHash::parse(text) {
                        self.links.entry(target).or_default().push(ci);
                    }
                }
                Value::Map(external) if key == "external" => {
                    for (name, value) in external {
                        if let (Value::Text(name), Value::Text(value)) = (name, value) {
                            self.external.insert((name.clone(), value.clone()), ci);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn cis(&self) -> impl Iterator<Item = (&CiHash, &str)> {
        self.kinds.iter().map(|(ci, kind)| (ci, kind.as_str()))
    }
    pub fn externals(&self) -> impl Iterator<Item = (&(String, String), &CiHash)> {
        self.external.iter()
    }
    pub fn kind_of(&self, ci: CiHash) -> Option<&str> {
        self.kinds.get(&ci).map(String::as_str)
    }

    pub fn attestations_for(&self, ci: CiHash) -> &[Attestation] {
        self.attestations.get(&ci).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn linked_to(&self, ci: CiHash) -> &[CiHash] {
        self.links.get(&ci).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn by_external(&self, key: &str, value: &str) -> Option<CiHash> {
        self.external.get(&(key.into(), value.into())).copied()
    }

    pub fn owner_of(&self, collection: &str) -> Option<Dgid> {
        self.collections.get(collection).copied()
    }

    pub fn collections(&self) -> impl Iterator<Item = (&String, &Dgid)> {
        self.collections.iter()
    }

    pub fn resolve(&self, ci: CiHash, trust: &Trust, at_least: TrustLevel) -> Option<BlobHash> {
        self.attestations_for(ci)
            .iter()
            .filter(|attestation| {
                attestation
                    .signer()
                    .is_some_and(|dgid| trust.trusts(dgid, at_least))
            })
            .find_map(|attestation| attestation.claim.blob.map(|blob| blob.hash()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spirit_core::record::{Claim, Tdr};
    use spirit_core::{BlobRef, Identity, TdHash};

    fn scratch(tag: &str) -> BlobStore {
        let dir = std::env::temp_dir().join(format!("spirit-index-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    #[derive(serde::Serialize)]
    struct CardBody {
        external: BTreeMap<String, String>,
        game: String,
        name: String,
    }

    #[derive(serde::Serialize)]
    struct PrintingBody {
        card: CiHash,
        set: String,
    }

    fn put_cir(store: &BlobStore, cir: &Cir) -> CiHash {
        CiHash::from_hash(store.put(&cir.encode().unwrap()).unwrap())
    }

    fn seed(store: &BlobStore, identity: &Identity) -> (CiHash, CiHash, BlobHash) {
        let mut external = BTreeMap::new();
        external.insert("scryfall_oracle_id".to_string(), "4457ed35".to_string());
        let card = put_cir(
            store,
            &Cir::new(
                "card",
                &CardBody {
                    external,
                    game: "mtg".into(),
                    name: "Lightning Bolt".into(),
                },
            )
            .unwrap(),
        );
        let printing = put_cir(
            store,
            &Cir::new(
                "card-printing",
                &PrintingBody {
                    card,
                    set: "hob".into(),
                },
            )
            .unwrap(),
        );
        let art = store.put(b"jpeg").unwrap();
        let td = TdHash::from_hash(
            store
                .put(
                    &Tdr::new("image-fetch", &("2026-09-04",))
                        .unwrap()
                        .encode()
                        .unwrap(),
                )
                .unwrap(),
        );
        let attestation = Attestation::sign(
            Claim::content(printing, td, BlobRef::from_hash(art)),
            identity,
        )
        .unwrap();
        let attestation_hash = store.put(&attestation.encode().unwrap()).unwrap();

        let head = Collection::new("catalog", "cards/mtg", identity.dgid()).with(
            Vec::new(),
            vec![card.hash(), printing.hash(), attestation_hash],
        );
        collection::publish(store, &head).unwrap();
        (card, printing, art)
    }

    #[test]
    fn the_fold_finds_kinds_links_and_external_ids() {
        let store = scratch("fold");
        let me = Identity::from_secret([1; 32]);
        let (card, printing, _) = seed(&store, &me);
        let index = Index::build(&store);

        assert_eq!(index.kind_of(card), Some("card"));
        assert_eq!(index.kind_of(printing), Some("card-printing"));
        assert_eq!(index.linked_to(card), &[printing]);
        assert_eq!(
            index.by_external("scryfall_oracle_id", "4457ed35"),
            Some(card)
        );
        assert_eq!(index.owner_of("cards/mtg"), Some(me.dgid()));
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn resolution_needs_a_trusted_signature() {
        let store = scratch("resolve");
        let me = Identity::from_secret([2; 32]);
        let (_, printing, art) = seed(&store, &me);
        let index = Index::build(&store);

        let trust = Trust::new().with_own(me.dgid());
        assert_eq!(
            index.resolve(printing, &trust, TrustLevel::Cache),
            Some(art)
        );

        let stranger = Trust::new().with_own(Identity::from_secret([3; 32]).dgid());
        assert_eq!(index.resolve(printing, &stranger, TrustLevel::Cache), None);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_legacy_ref_is_absorbed_without_a_collection() {
        let store = scratch("legacy");
        let me = Identity::from_secret([4; 32]);
        let cir = Cir::new(
            "card",
            &CardBody {
                external: BTreeMap::new(),
                game: "mtg".into(),
                name: "Shock".into(),
            },
        )
        .unwrap();
        let ci = put_cir(&store, &cir);
        let manifest = spirit_core::canonical::to_vec(&vec![ci.hash().to_string()]).unwrap();
        let manifest_hash = store.put(&manifest).unwrap();
        spirit_core::refs::write(&store, "legacy", manifest_hash).unwrap();

        let index = Index::build(&store);
        assert_eq!(index.kind_of(ci), Some("card"));
        assert!(index.owner_of("legacy").is_none());
        let _ = me;
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn an_empty_store_indexes_to_nothing() {
        let store = scratch("empty");
        let index = Index::build(&store);
        assert_eq!(index.collections().count(), 0);
        assert!(index
            .attestations_for(CiHash::from_hash(BlobHash::of(b"x")))
            .is_empty());
        let _ = std::fs::remove_dir_all(store.root());
    }
}
