use ciborium::value::Value;
use serde::Serialize;
use spirit_core::collection::{self, Item, Op, OpKind};
use spirit_core::record::{Attestation, Cir, Claim, ClaimKind, Tdr};
use spirit_core::{
    canonical, identity, refs, AttHash, BlobHash, BlobRef, BlobStore, CiHash, Dgid, Identity,
    TdHash, Trust, TrustLevel,
};
use spirit_index::Index;
use spirit_routing::artifacts as rank_artifacts;
use spirit_routing::{resolve, Policy};
use std::error::Error;
use std::path::{Path, PathBuf};

pub struct Local {
    pub dir: PathBuf,
    pub store: BlobStore,
    pub identity: Identity,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityInfo {
    pub store: String,
    pub dgid: String,
    pub node_id: String,
    pub key_path: String,
    pub node_key_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlobInfo {
    pub hash: String,
    pub size: u64,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefInfo {
    pub name: String,
    pub hash: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordView {
    pub hash: String,
    pub address: String,
    pub kind: String,
    pub signer: Option<String>,
    pub verified: bool,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttestationView {
    pub hash: String,
    pub kind: String,
    pub ci: String,
    pub td: Option<String>,
    pub blob: Option<String>,
    pub other: Option<String>,
    pub expires: Option<String>,
    pub signer: Option<String>,
    pub level: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemView {
    pub ci: String,
    pub kind: String,
    pub label: Option<String>,
    pub default_td: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionSummary {
    pub name: String,
    pub head: String,
    pub owner: String,
    pub items: usize,
    pub attestations: usize,
    pub closure: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionView {
    pub name: String,
    pub head: String,
    pub owner: String,
    pub forked_from: Option<String>,
    pub items: Vec<ItemView>,
    pub attestations: Vec<AttestationView>,
    pub ops: usize,
    pub records: usize,
    pub closure: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpView {
    pub seq: u64,
    pub hash: String,
    pub kind: String,
    pub target: String,
    pub counted: bool,
}

#[derive(Debug, Clone)]
pub enum Edit {
    Add {
        ci: CiHash,
        label: Option<String>,
        default_td: Option<TdHash>,
    },
    Remove {
        ci: CiHash,
    },
    Attest {
        hash: BlobHash,
    },
    Record {
        hash: BlobHash,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityEntry {
    pub ci: String,
    pub kind: String,
    pub linked_from: Vec<String>,
    pub attestations: Vec<AttestationView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionOwner {
    pub name: String,
    pub owner: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalId {
    pub key: String,
    pub value: String,
    pub ci: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexView {
    pub collections: Vec<CollectionOwner>,
    pub identities: Vec<IdentityEntry>,
    pub externals: Vec<ExternalId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub blob: String,
    pub td: String,
    pub td_kind: String,
    pub variant: Option<String>,
    pub snapshot: Option<String>,
    pub signer: Option<String>,
    pub level: String,
    pub held: bool,
    pub expires: Option<String>,
    pub expired: bool,
    pub eligible: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pick {
    pub blob: String,
    pub signer: String,
    pub held: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactView {
    pub ci: String,
    pub kind: String,
    pub body: Option<serde_json::Value>,
    pub minimum: String,
    pub prefer_td: Option<String>,
    pub artifacts: Vec<Artifact>,
    pub relations: Vec<AttestationView>,
    pub pick: Option<Pick>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberView {
    pub node_id: String,
    pub ci: String,
    pub role: Option<String>,
    pub tags: Vec<String>,
    pub expires: Option<String>,
    pub verified: bool,
    pub expired: bool,
    pub this_device: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrustEntry {
    pub dgid: String,
    pub level: String,
}

impl Local {
    pub fn open(dir: &Path) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            dir: dir.to_path_buf(),
            store: BlobStore::open(dir)?,
            identity: identity::load_or_create(dir)?,
        })
    }

    pub fn trust(&self) -> Trust {
        Trust::load(&self.dir).with_own(self.identity.dgid())
    }

    pub fn identity_info(&self) -> IdentityInfo {
        let dgid = self.identity.dgid();
        let node_id = identity::load_or_create_device(&self.dir)
            .map(|device| {
                device
                    .dgid()
                    .to_string()
                    .trim_start_matches("dgid:")
                    .to_string()
            })
            .unwrap_or_default();
        IdentityInfo {
            store: self.dir.display().to_string(),
            dgid: dgid.to_string(),
            node_id,
            key_path: identity::key_path(&self.dir).display().to_string(),
            node_key_path: identity::device_key_path(&self.dir).display().to_string(),
        }
    }

    pub fn blobs(&self) -> Vec<BlobInfo> {
        let mut rows = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.store.root()) else {
            return rows;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(hash) = BlobHash::parse(&name) else {
                continue;
            };
            let Ok(bytes) = self.store.get(hash) else {
                continue;
            };
            rows.push(BlobInfo {
                hash: name,
                size: bytes.len() as u64,
                kind: describe(&bytes),
            });
        }
        rows.sort_by(|left, right| left.hash.cmp(&right.hash));
        rows
    }

    pub fn refs(&self) -> Vec<RefInfo> {
        refs::list(&self.store)
            .into_iter()
            .map(|(name, hash)| RefInfo {
                name,
                hash: hash.to_string(),
                kind: self
                    .store
                    .get(hash)
                    .map(|bytes| describe(&bytes))
                    .unwrap_or_else(|_| "missing".into()),
            })
            .collect()
    }

    pub fn record(&self, hash: BlobHash) -> Result<RecordView, Box<dyn Error>> {
        let bytes = self.store.get(hash)?;
        let value: Value = canonical::from_slice(&bytes)
            .map_err(|_| format!("{hash} holds {} bytes that are not a record", bytes.len()))?;
        let kind = describe(&bytes);
        let (signer, verified) = if let Ok(attestation) = Attestation::decode(&bytes) {
            signer_of(
                attestation.proof.as_ref().map(|p| p.dgid),
                attestation.signer(),
            )
        } else if let Ok(op) = Op::decode(&bytes) {
            signer_of(op.proof.as_ref().map(|p| p.dgid), op.signer())
        } else {
            (None, false)
        };
        Ok(RecordView {
            hash: hash.to_string(),
            address: address_of(&kind, hash),
            kind,
            signer,
            verified,
            body: cbor_to_json(&value),
        })
    }

    pub fn mint_cir(&self, kind: &str, body: &serde_json::Value) -> Result<CiHash, Box<dyn Error>> {
        check_body(body)?;
        let cir = Cir::new(kind, body)?;
        Ok(CiHash::from_hash(self.store.put(&cir.encode()?)?))
    }

    pub fn mint_tdr(&self, kind: &str, body: &serde_json::Value) -> Result<TdHash, Box<dyn Error>> {
        check_body(body)?;
        let tdr = Tdr::new(kind, body)?;
        Ok(TdHash::from_hash(self.store.put(&tdr.encode()?)?))
    }

    pub fn attest_content(
        &self,
        ci: CiHash,
        td: TdHash,
        blob: BlobHash,
    ) -> Result<AttHash, Box<dyn Error>> {
        if !self.store.has(ci.hash()) {
            return Err(format!("{ci} is not in the store").into());
        }
        if !self.store.has(td.hash()) {
            return Err(format!("{td} is not in the store").into());
        }
        Cir::decode(&self.store.get(ci.hash())?).map_err(|_| format!("{ci} is not a CIR"))?;
        Tdr::decode(&self.store.get(td.hash())?).map_err(|_| format!("{td} is not a TDR"))?;
        let attestation = Attestation::sign(
            Claim::content(ci, td, BlobRef::from_hash(blob)),
            &self.identity,
        )?;
        Ok(AttHash::from_hash(self.store.put(&attestation.encode()?)?))
    }

    pub fn attest_relation(
        &self,
        relation: &str,
        ci: CiHash,
        other: CiHash,
    ) -> Result<AttHash, Box<dyn Error>> {
        let kind = match relation {
            "same-as" => ClaimKind::SameAs,
            "superseded-by" => ClaimKind::SupersededBy,
            "previous-version" => ClaimKind::PreviousVersion,
            other => {
                return Err(format!(
                    "{other:?} is not a relation; use same-as, superseded-by or previous-version"
                )
                .into())
            }
        };
        let attestation = Attestation::sign(Claim::relation(kind, ci, other), &self.identity)?;
        Ok(AttHash::from_hash(self.store.put(&attestation.encode()?)?))
    }

    pub fn collections(&self) -> Vec<CollectionSummary> {
        refs::list(&self.store)
            .into_iter()
            .filter_map(|(name, hash)| {
                let (head, ops) = collection::load(&self.store, &name)?;
                let items = collection::fold(head.owner, &ops);
                Some(CollectionSummary {
                    name,
                    head: hash.to_string(),
                    owner: head.owner.to_string(),
                    items: items.len(),
                    attestations: head.attestations.len(),
                    closure: head.refs.len(),
                })
            })
            .collect()
    }

    pub fn collection(&self, name: &str) -> Result<CollectionView, Box<dyn Error>> {
        let head_hash = refs::read(&self.store, name).ok_or_else(|| format!("no ref {name}"))?;
        let (head, ops) = collection::load(&self.store, name)
            .ok_or_else(|| format!("{name} is not a collection"))?;
        let trust = self.trust();
        let items = collection::fold(head.owner, &ops)
            .into_iter()
            .map(|item| ItemView {
                ci: item.ci.to_string(),
                kind: self.cir_kind(item.ci),
                label: item.label,
                default_td: item.default_td.map(|td| td.to_string()),
            })
            .collect();
        let attestations = head
            .attestations
            .iter()
            .map(|hash| match self.load_attestation(*hash) {
                Some(attestation) => attestation_view(*hash, &attestation, &trust),
                None => missing_attestation(*hash),
            })
            .collect();
        Ok(CollectionView {
            name: name.to_string(),
            head: head_hash.to_string(),
            owner: head.owner.to_string(),
            forked_from: head.forked_from.map(|dgid| dgid.to_string()),
            items,
            attestations,
            ops: head.ops.len(),
            records: head.records.len(),
            closure: head.refs.len(),
        })
    }

    pub fn collection_ops(&self, name: &str) -> Result<Vec<OpView>, Box<dyn Error>> {
        let (head, ops) = collection::load(&self.store, name)
            .ok_or_else(|| format!("{name} is not a collection"))?;
        let mut ordered: Vec<(u64, BlobHash, &Op)> = ops
            .iter()
            .filter_map(|op| op.address().ok().map(|hash| (op.claim.seq, hash, op)))
            .collect();
        ordered.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        Ok(ordered
            .into_iter()
            .map(|(seq, hash, op)| OpView {
                seq,
                hash: hash.to_string(),
                kind: match op.claim.kind {
                    OpKind::Add => "add",
                    OpKind::Remove => "remove",
                    OpKind::Reorder => "reorder",
                }
                .into(),
                target: match op.claim.kind {
                    OpKind::Add => op
                        .claim
                        .item
                        .as_ref()
                        .map(|item| item.ci.to_string())
                        .unwrap_or_default(),
                    OpKind::Remove => op.claim.target.map(|ci| ci.to_string()).unwrap_or_default(),
                    OpKind::Reorder => format!(
                        "{} item(s)",
                        op.claim.order.as_ref().map(Vec::len).unwrap_or(0)
                    ),
                },
                counted: op.claim.owner == head.owner && op.owned(),
            })
            .collect())
    }

    pub fn edit_collection(&self, name: &str, edit: Edit) -> Result<BlobHash, Box<dyn Error>> {
        if !refs::valid_name(name) {
            return Err(format!("{name:?} is not a usable collection name").into());
        }
        let mut builder = collection::Builder::open(&self.store, name, &self.identity);
        match edit {
            Edit::Add {
                ci,
                label,
                default_td,
            } => {
                if !self.store.has(ci.hash()) {
                    return Err(format!("{ci} is not in the store").into());
                }
                Cir::decode(&self.store.get(ci.hash())?)
                    .map_err(|_| format!("{ci} is not a CIR"))?;
                builder
                    .add(Item {
                        ci,
                        label,
                        default_td,
                    })
                    .record(ci.hash());
            }
            Edit::Remove { ci } => {
                builder.remove(ci);
            }
            Edit::Attest { hash } => {
                self.load_attestation(hash)
                    .ok_or_else(|| format!("{hash} is not an attestation in the store"))?;
                builder.attest(hash).record(hash);
            }
            Edit::Record { hash } => {
                if !self.store.has(hash) {
                    return Err(format!("{hash} is not in the store").into());
                }
                builder.record(hash);
            }
        }
        Ok(builder.publish(&self.store, &self.identity)?)
    }

    pub fn index(&self) -> IndexView {
        let index = Index::build(&self.store);
        let trust = self.trust();
        IndexView {
            collections: index
                .collections()
                .map(|(name, owner)| CollectionOwner {
                    name: name.clone(),
                    owner: owner.to_string(),
                })
                .collect(),
            identities: index
                .cis()
                .map(|(ci, kind)| IdentityEntry {
                    ci: ci.to_string(),
                    kind: kind.to_string(),
                    linked_from: index
                        .linked_to(*ci)
                        .iter()
                        .map(|other| other.to_string())
                        .collect(),
                    attestations: index
                        .attestations_for(*ci)
                        .iter()
                        .map(|attestation| {
                            let hash = attestation.address().map(|h| h.hash()).ok();
                            attestation_view(hash.unwrap_or(BlobHash::of(b"")), attestation, &trust)
                        })
                        .collect(),
                })
                .collect(),
            externals: index
                .externals()
                .map(|((key, value), ci)| ExternalId {
                    key: key.clone(),
                    value: value.clone(),
                    ci: ci.to_string(),
                })
                .collect(),
        }
    }

    pub fn artifacts(
        &self,
        ci: CiHash,
        minimum: TrustLevel,
        prefer_td: Option<TdHash>,
    ) -> ArtifactView {
        let index = Index::build(&self.store);
        let trust = self.trust();
        let policy = Policy {
            minimum,
            prefer_held: true,
            prefer_td,
        };
        let body = self
            .store
            .get(ci.hash())
            .ok()
            .and_then(|bytes| Cir::decode(&bytes).ok())
            .map(|cir| cbor_to_json(&cir.body));
        let artifacts = rank_artifacts(&self.store, &index, &trust, ci, policy)
            .into_iter()
            .map(|candidate| Artifact {
                blob: candidate.blob.to_string(),
                td: candidate.td.map(|td| td.to_string()).unwrap_or_default(),
                td_kind: candidate.td_kind.unwrap_or_else(|| "missing".into()),
                variant: candidate.variant,
                snapshot: candidate.snapshot,
                signer: candidate.signer.map(|dgid| dgid.to_string()),
                level: candidate.level.label().into(),
                held: candidate.held,
                expires: candidate.expires,
                expired: candidate.expired,
                eligible: candidate.eligible,
            })
            .collect();
        let relations = index
            .attestations_for(ci)
            .iter()
            .filter(|attestation| attestation.claim.kind != ClaimKind::Content)
            .map(|attestation| {
                let hash = attestation
                    .address()
                    .map(|h| h.hash())
                    .unwrap_or(BlobHash::of(b""));
                attestation_view(hash, attestation, &trust)
            })
            .collect();
        let pick = resolve(&self.store, &index, &trust, ci, policy).map(|resolution| Pick {
            blob: resolution.blob.to_string(),
            signer: resolution.signer.to_string(),
            held: resolution.held,
        });
        ArtifactView {
            ci: ci.to_string(),
            kind: index
                .kind_of(ci)
                .map(String::from)
                .unwrap_or_else(|| "not indexed".into()),
            body,
            minimum: minimum.label().into(),
            prefer_td: prefer_td.map(|td| td.to_string()),
            artifacts,
            relations,
            pick,
        }
    }

    pub fn members(&self) -> Vec<MemberView> {
        let me = identity::load_device(&self.dir).map(|device| {
            device
                .dgid()
                .to_string()
                .trim_start_matches("dgid:")
                .to_string()
        });
        spirit_schema::device::members(&self.store, self.identity.dgid())
            .into_iter()
            .map(|member| MemberView {
                node_id: member.device.node_hex(),
                ci: member.ci.to_string(),
                role: member.device.settings.role.clone(),
                tags: member.device.settings.tags.clone(),
                expires: member.device.settings.expires.clone(),
                verified: member.verified,
                expired: member.expired,
                this_device: me.as_deref() == Some(member.device.node_hex().as_str()),
            })
            .collect()
    }

    pub fn revoke(&self, node_hex: &str) -> Result<Option<BlobHash>, Box<dyn Error>> {
        Ok(spirit_schema::device::revoke(
            &self.store,
            &self.identity,
            node_hex.trim_start_matches("nodeid:"),
        )?)
    }

    pub fn lock(&self, tdr: &Tdr) -> Result<spirit_routing::transform::Locked, Box<dyn Error>> {
        let index = Index::build(&self.store);
        let trust = self.trust();
        Ok(spirit_routing::transform::lock(
            &self.store,
            &index,
            &trust,
            None,
            tdr,
        )?)
    }

    pub fn run(
        &self,
        td: TdHash,
        output: CiHash,
    ) -> Result<spirit_routing::transform::Ran, Box<dyn Error>> {
        Ok(spirit_routing::transform::run(
            &self.store,
            &self.identity,
            None,
            None,
            td,
            output,
        )?)
    }

    pub fn trust_entries(&self) -> Vec<TrustEntry> {
        let trust = self.trust();
        let mut entries = vec![TrustEntry {
            dgid: self.identity.dgid().to_string(),
            level: "mesh".into(),
        }];
        entries.extend(trust.entries().map(|(dgid, level)| TrustEntry {
            dgid: dgid.to_string(),
            level: level.label().into(),
        }));
        entries
    }

    pub fn set_trust(&self, who: &str, level: TrustLevel) -> Result<Dgid, Box<dyn Error>> {
        let dgid = crate::mesh::dgid_of(who)
            .or_else(|| Dgid::parse(who))
            .ok_or("neither a node id nor a dgid")?;
        let mut trust = self.trust();
        trust.set(dgid, level);
        trust.save(&self.dir)?;
        Ok(dgid)
    }

    fn cir_kind(&self, ci: CiHash) -> String {
        self.store
            .get(ci.hash())
            .ok()
            .and_then(|bytes| Cir::decode(&bytes).ok())
            .map(|cir| cir.kind)
            .unwrap_or_else(|| "missing".into())
    }

    fn load_attestation(&self, hash: BlobHash) -> Option<Attestation> {
        Attestation::decode(&self.store.get(hash).ok()?).ok()
    }
}

fn check_body(body: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    if !body.is_object() {
        return Err("a record body is a JSON object".into());
    }
    Ok(())
}

fn signer_of(claimed: Option<Dgid>, verified: Option<Dgid>) -> (Option<String>, bool) {
    match (claimed, verified) {
        (_, Some(dgid)) => (Some(dgid.to_string()), true),
        (Some(dgid), None) => (Some(dgid.to_string()), false),
        (None, None) => (None, false),
    }
}

pub fn claim_kind_label(kind: ClaimKind) -> &'static str {
    match kind {
        ClaimKind::Content => "content",
        ClaimKind::SameAs => "same-as",
        ClaimKind::SupersededBy => "superseded-by",
        ClaimKind::PreviousVersion => "previous-version",
    }
}

fn attestation_view(hash: BlobHash, attestation: &Attestation, trust: &Trust) -> AttestationView {
    let signer = attestation.signer();
    AttestationView {
        hash: hash.to_string(),
        kind: claim_kind_label(attestation.claim.kind).into(),
        ci: attestation.claim.ci.to_string(),
        td: attestation.claim.td.map(|td| td.to_string()),
        blob: attestation.claim.blob.map(|blob| blob.to_string()),
        other: attestation.claim.other.map(|ci| ci.to_string()),
        expires: attestation.claim.expires.clone(),
        signer: signer.map(|dgid| dgid.to_string()),
        level: signer
            .map(|dgid| trust.level(dgid).label())
            .unwrap_or("unsigned")
            .into(),
    }
}

fn missing_attestation(hash: BlobHash) -> AttestationView {
    AttestationView {
        hash: hash.to_string(),
        kind: "missing".into(),
        ci: String::new(),
        td: None,
        blob: None,
        other: None,
        expires: None,
        signer: None,
        level: "unsigned".into(),
    }
}

pub fn address_of(kind: &str, hash: BlobHash) -> String {
    let prefix = match kind.split(' ').next().unwrap_or("") {
        "cir" => "ci:",
        "tdr" => "td:",
        "attestation" => "att:",
        "collection" => "col:",
        _ => "",
    };
    format!("{prefix}{hash}")
}

pub fn parse_hash(text: &str) -> Result<BlobHash, String> {
    let text = text.trim();
    let bare = ["ci:", "td:", "att:", "col:", "blob:"]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))
        .unwrap_or(text);
    BlobHash::parse(bare).ok_or_else(|| format!("{text:?} is not a hash"))
}

pub fn describe(bytes: &[u8]) -> String {
    let Ok(value) = canonical::from_slice::<Value>(bytes) else {
        return "bytes".into();
    };
    let Value::Map(entries) = &value else {
        return "bytes".into();
    };
    let field = |name: &str| {
        entries.iter().find_map(|(key, item)| match (key, item) {
            (Value::Text(key), Value::Text(text)) if key == name => Some(text.clone()),
            _ => None,
        })
    };
    match (field("record"), field("kind")) {
        (Some(record), Some(kind)) => format!("{record} ({kind})"),
        (Some(record), None) => record,
        (None, Some(kind)) => kind,
        (None, None) => "cbor map".into(),
    }
}

pub fn cbor_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(flag) => serde_json::Value::Bool(*flag),
        Value::Integer(number) => {
            let number: i128 = (*number).into();
            i64::try_from(number)
                .map(|n| serde_json::Value::Number(n.into()))
                .or_else(|_| u64::try_from(number).map(|n| serde_json::Value::Number(n.into())))
                .unwrap_or_else(|_| serde_json::Value::String(number.to_string()))
        }
        Value::Float(number) => serde_json::Number::from_f64(*number)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Bytes(bytes) => {
            serde_json::Value::String(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
        }
        Value::Text(text) => serde_json::Value::String(text.clone()),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(cbor_to_json).collect()),
        Value::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(key, item)| {
                    let key = match key {
                        Value::Text(text) => text.clone(),
                        other => format!("{other:?}"),
                    };
                    (key, cbor_to_json(item))
                })
                .collect(),
        ),
        Value::Tag(_, inner) => cbor_to_json(inner),
        _ => serde_json::Value::Null,
    }
}

pub fn gateway_call(
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(u16, Vec<u8>), Box<dyn Error>> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
    let body = body.unwrap_or(&[]);
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed gateway reply")?;
    let status_line = String::from_utf8_lossy(&raw[..split]);
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or("gateway reply has no status")?;
    Ok((status, raw[split + 4..].to_vec()))
}

pub fn gateway_alive(port: u16) -> bool {
    gateway_call(port, "GET", "/gateway/status", None, None)
        .map(|(status, body)| status == 200 && body.windows(7).any(|w| w == b"node_id"))
        .unwrap_or(false)
}

pub type DaemonReply = Result<(u16, Vec<u8>), Box<dyn Error>>;

pub fn daemon_call(dir: &Path, method: &str, path: &str, body: &[u8]) -> Option<DaemonReply> {
    if crate::api::path(dir).exists() {
        let reply = crate::api::call(
            dir,
            &crate::api::ApiRequest {
                method: method.into(),
                path: path.into(),
                query: String::new(),
                body: body.to_vec(),
            },
        );
        match reply {
            Ok(reply) => return Some(Ok((reply.status, reply.body))),
            Err(error) if crate::lock::holder(dir).is_some() => return Some(Err(error)),
            Err(_) => {}
        }
    }
    let port = crate::gateway::read_port(dir).filter(|port| gateway_alive(*port))?;
    let token = std::fs::read_to_string(crate::gateway::token_path(dir)).ok()?;
    Some(gateway_call(
        port,
        method,
        path,
        Some(token.trim()),
        Some(body),
    ))
}
