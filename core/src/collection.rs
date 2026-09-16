use crate::address::{CiHash, ColHash, TdHash};
use crate::canonical::{self, CanonError};
use crate::identity::{self, Dgid, Identity};
use crate::record::Proof;
use crate::store::{BlobHash, BlobStore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const COLLECTION_RECORD: &str = "collection";
pub const COLLECTION_KIND: &str = "collection";
pub const OP_KIND: &str = "collection-op";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpKind {
    Add,
    Remove,
    Reorder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub ci: CiHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_td: Option<TdHash>,
}

impl Item {
    pub fn new(ci: CiHash) -> Self {
        Self {
            ci,
            label: None,
            default_td: None,
        }
    }

    pub fn labelled(ci: CiHash, label: impl Into<String>) -> Self {
        Self {
            ci,
            label: Some(label.into()),
            default_td: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpClaim {
    pub kind: OpKind,
    pub collection: String,
    pub owner: Dgid,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Item>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<CiHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<CiHash>>,
}

impl OpClaim {
    pub fn scope(&self) -> Result<Vec<u8>, CanonError> {
        Ok(canonical::hash(self)?.as_bytes().to_vec())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub record: String,
    pub claim: OpClaim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Proof>,
}

impl Op {
    pub fn add(
        collection: &str,
        seq: u64,
        item: Item,
        identity: &Identity,
    ) -> Result<Self, CanonError> {
        Self::sign(
            OpClaim {
                kind: OpKind::Add,
                collection: collection.into(),
                owner: identity.dgid(),
                seq,
                item: Some(item),
                target: None,
                order: None,
            },
            identity,
        )
    }

    pub fn remove(
        collection: &str,
        seq: u64,
        target: CiHash,
        identity: &Identity,
    ) -> Result<Self, CanonError> {
        Self::sign(
            OpClaim {
                kind: OpKind::Remove,
                collection: collection.into(),
                owner: identity.dgid(),
                seq,
                item: None,
                target: Some(target),
                order: None,
            },
            identity,
        )
    }

    pub fn reorder(
        collection: &str,
        seq: u64,
        order: Vec<CiHash>,
        identity: &Identity,
    ) -> Result<Self, CanonError> {
        Self::sign(
            OpClaim {
                kind: OpKind::Reorder,
                collection: collection.into(),
                owner: identity.dgid(),
                seq,
                item: None,
                target: None,
                order: Some(order),
            },
            identity,
        )
    }

    pub fn sign(claim: OpClaim, identity: &Identity) -> Result<Self, CanonError> {
        let sig = identity.sign(&claim.scope()?);
        Ok(Self {
            record: OP_KIND.into(),
            claim,
            proof: Some(Proof {
                dgid: identity.dgid(),
                sig,
            }),
        })
    }

    pub fn signer(&self) -> Option<Dgid> {
        let proof = self.proof.as_ref()?;
        let scope = self.claim.scope().ok()?;
        identity::verify(proof.dgid, &scope, proof.sig).then_some(proof.dgid)
    }

    pub fn owned(&self) -> bool {
        self.signer() == Some(self.claim.owner)
    }

    pub fn encode(&self) -> Result<Vec<u8>, CanonError> {
        canonical::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CanonError> {
        let op: Self = canonical::from_slice(bytes)?;
        if op.record != OP_KIND {
            return Err(CanonError::Decode(format!(
                "expected a {OP_KIND} record, found {:?}",
                op.record
            )));
        }
        Ok(op)
    }

    pub fn address(&self) -> Result<BlobHash, CanonError> {
        canonical::hash(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub record: String,
    pub kind: String,
    pub name: String,
    pub owner: Dgid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<Dgid>,
    pub ops: Vec<BlobHash>,
    #[serde(default)]
    pub attestations: Vec<BlobHash>,
    #[serde(default)]
    pub records: Vec<BlobHash>,
    pub refs: Vec<BlobHash>,
}

impl Collection {
    pub fn new(kind: &str, name: &str, owner: Dgid) -> Self {
        Self {
            record: COLLECTION_RECORD.into(),
            kind: kind.into(),
            name: name.into(),
            owner,
            forked_from: None,
            ops: Vec::new(),
            attestations: Vec::new(),
            records: Vec::new(),
            refs: Vec::new(),
        }
    }

    pub fn with(mut self, ops: Vec<BlobHash>, records: Vec<BlobHash>) -> Self {
        self.ops = ops;
        self.records = records;
        self.reindex()
    }

    pub fn attesting(mut self, attestations: Vec<BlobHash>) -> Self {
        self.attestations = attestations;
        self.reindex()
    }

    fn reindex(mut self) -> Self {
        let mut refs: Vec<BlobHash> = self
            .ops
            .iter()
            .chain(self.attestations.iter())
            .chain(self.records.iter())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        refs.sort();
        self.refs = refs;
        self
    }

    pub fn is_legacy(&self) -> bool {
        self.record.is_empty()
    }

    pub fn encode(&self) -> Result<Vec<u8>, CanonError> {
        canonical::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CanonError> {
        let head: Self = canonical::from_slice(bytes)?;
        let modern = head.record == COLLECTION_RECORD;
        let legacy = head.record.is_empty() && head.kind == COLLECTION_KIND;
        if !modern && !legacy {
            return Err(CanonError::Decode(format!(
                "expected a {COLLECTION_RECORD} record, found record {:?} kind {:?}",
                head.record, head.kind
            )));
        }
        Ok(head)
    }

    pub fn address(&self) -> Result<ColHash, CanonError> {
        canonical::hash(self).map(ColHash::from_hash)
    }

    fn contains_all(&self, other: &Collection) -> bool {
        let mine: BTreeSet<BlobHash> = self.refs.iter().copied().collect();
        other.refs.iter().all(|hash| mine.contains(hash))
    }
}

#[derive(Debug, Clone)]
enum Draft {
    Add(Item),
    Remove(CiHash),
}

impl Draft {
    fn ci(&self) -> CiHash {
        match self {
            Draft::Add(item) => item.ci,
            Draft::Remove(ci) => *ci,
        }
    }
}

pub struct Builder {
    kind: String,
    name: String,
    owner: Dgid,
    forked_from: Option<Dgid>,
    items: Vec<Item>,
    baseline: BTreeSet<CiHash>,
    ops: Vec<BlobHash>,
    next_seq: u64,
    pending: Vec<Draft>,
    attestations: Vec<BlobHash>,
    records: Vec<BlobHash>,
}

impl Builder {
    pub fn open(store: &BlobStore, name: &str, identity: &Identity) -> Self {
        let me = identity.dgid();
        let Some((head, ops)) = load(store, name) else {
            return Self {
                kind: COLLECTION_KIND.into(),
                name: name.into(),
                owner: me,
                forked_from: None,
                items: Vec::new(),
                baseline: BTreeSet::new(),
                ops: Vec::new(),
                next_seq: 1,
                pending: Vec::new(),
                attestations: Vec::new(),
                records: Vec::new(),
            };
        };
        let items = fold(head.owner, &ops);
        let next_seq = ops.iter().map(|op| op.claim.seq).max().unwrap_or(0) + 1;
        let forking = head.owner != me;
        let pending = if forking {
            items.iter().cloned().map(Draft::Add).collect()
        } else {
            Vec::new()
        };
        let baseline = if forking {
            BTreeSet::new()
        } else {
            items.iter().map(|item| item.ci).collect()
        };
        Self {
            kind: head.kind.clone(),
            name: name.into(),
            owner: me,
            forked_from: forking.then_some(head.owner),
            items,
            baseline,
            ops: head.ops.clone(),
            next_seq,
            pending,
            attestations: head.attestations.clone(),
            records: head.records.clone(),
        }
    }

    pub fn kind(&mut self, kind: &str) -> &mut Self {
        self.kind = kind.into();
        self
    }

    pub fn add(&mut self, item: Item) -> &mut Self {
        match self.items.iter_mut().find(|held| held.ci == item.ci) {
            Some(held) => *held = item.clone(),
            None => self.items.push(item.clone()),
        }
        self.pending.retain(|draft| draft.ci() != item.ci);
        self.pending.push(Draft::Add(item));
        self
    }

    pub fn remove(&mut self, ci: CiHash) -> &mut Self {
        self.items.retain(|held| held.ci != ci);
        self.pending.retain(|draft| draft.ci() != ci);
        if self.baseline.contains(&ci) {
            self.pending.push(Draft::Remove(ci));
        }
        self
    }

    pub fn record(&mut self, hash: BlobHash) -> &mut Self {
        if !self.records.contains(&hash) {
            self.records.push(hash);
        }
        self
    }

    pub fn attest(&mut self, hash: BlobHash) -> &mut Self {
        self.attestations.retain(|held| *held != hash);
        self.attestations.push(hash);
        self
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn attestations(&self) -> &[BlobHash] {
        &self.attestations
    }

    pub fn forked_from(&self) -> Option<Dgid> {
        self.forked_from
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn publish(&mut self, store: &BlobStore, identity: &Identity) -> Result<BlobHash, String> {
        if identity.dgid() != self.owner {
            return Err(format!(
                "collection {} is built by the identity that opened it",
                self.name
            ));
        }
        let mut ops = self.ops.clone();
        for draft in std::mem::take(&mut self.pending) {
            let seq = self.next_seq;
            self.next_seq += 1;
            let op = match draft {
                Draft::Add(item) => Op::add(&self.name, seq, item, identity),
                Draft::Remove(ci) => Op::remove(&self.name, seq, ci, identity),
            }
            .map_err(|e| e.to_string())?;
            let hash = put_op(store, &op)?;
            if !ops.contains(&hash) {
                ops.push(hash);
            }
        }
        self.ops = ops.clone();
        self.baseline = self.items.iter().map(|item| item.ci).collect();
        let mut head = Collection::new(&self.kind, &self.name, identity.dgid())
            .with(ops, self.records.clone())
            .attesting(self.attestations.clone());
        head.forked_from = self.forked_from;
        publish(store, &head)
    }
}

pub fn load(store: &BlobStore, name: &str) -> Option<(Collection, Vec<Op>)> {
    let head = Collection::decode(&store.get(crate::refs::read(store, name)?).ok()?).ok()?;
    let ops = load_ops(store, &head);
    Some((head, ops))
}

fn load_ops(store: &BlobStore, head: &Collection) -> Vec<Op> {
    head.ops
        .iter()
        .filter_map(|hash| Op::decode(&store.get(*hash).ok()?).ok())
        .collect()
}

pub fn publish(store: &BlobStore, head: &Collection) -> Result<BlobHash, String> {
    let bytes = head.encode().map_err(|e| e.to_string())?;
    let hash = store.put(&bytes).map_err(|e| e.to_string())?;
    crate::refs::write(store, &head.name, hash)?;
    Ok(hash)
}

pub fn put_op(store: &BlobStore, op: &Op) -> Result<BlobHash, String> {
    let bytes = op.encode().map_err(|e| e.to_string())?;
    store.put(&bytes).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Merged {
    Unchanged,
    Adopted(BlobHash),
    Union(BlobHash),
    Replaced(BlobHash),
}

impl Merged {
    pub fn head(&self) -> Option<BlobHash> {
        match self {
            Merged::Unchanged => None,
            Merged::Adopted(hash) | Merged::Union(hash) | Merged::Replaced(hash) => Some(*hash),
        }
    }
}

pub fn merge(store: &BlobStore, name: &str, incoming_hash: BlobHash) -> Result<Merged, String> {
    let incoming_bytes = store
        .get(incoming_hash)
        .map_err(|e| format!("incoming head {incoming_hash}: {e}"))?;
    let Ok(incoming) = Collection::decode(&incoming_bytes) else {
        crate::refs::write(store, name, incoming_hash)?;
        return Ok(Merged::Replaced(incoming_hash));
    };
    let Some(local_hash) = crate::refs::read(store, name) else {
        crate::refs::write(store, name, incoming_hash)?;
        return Ok(Merged::Replaced(incoming_hash));
    };
    if local_hash == incoming_hash {
        return Ok(Merged::Unchanged);
    }
    let local = store
        .get(local_hash)
        .ok()
        .and_then(|bytes| Collection::decode(&bytes).ok());
    let Some(local) = local else {
        crate::refs::write(store, name, incoming_hash)?;
        return Ok(Merged::Replaced(incoming_hash));
    };
    if local.owner != incoming.owner || local.name != incoming.name {
        crate::refs::write(store, name, incoming_hash)?;
        return Ok(Merged::Replaced(incoming_hash));
    }
    if local.contains_all(&incoming) {
        return Ok(Merged::Unchanged);
    }
    if incoming.contains_all(&local) {
        crate::refs::write(store, name, incoming_hash)?;
        return Ok(Merged::Adopted(incoming_hash));
    }
    let union = |mine: &[BlobHash], theirs: &[BlobHash]| -> Vec<BlobHash> {
        let mut out = mine.to_vec();
        for hash in theirs {
            if !out.contains(hash) {
                out.push(*hash);
            }
        }
        out
    };
    let mut head = Collection::new(&incoming.kind, &local.name, local.owner)
        .with(
            union(&local.ops, &incoming.ops),
            union(&local.records, &incoming.records),
        )
        .attesting(union(&local.attestations, &incoming.attestations));
    head.forked_from = local.forked_from.or(incoming.forked_from);
    publish(store, &head).map(Merged::Union)
}

pub fn fold(owner: Dgid, ops: &[Op]) -> Vec<Item> {
    let mut ordered: Vec<(u64, BlobHash, &Op)> = ops
        .iter()
        .filter(|op| op.claim.owner == owner && op.owned())
        .filter_map(|op| op.address().ok().map(|hash| (op.claim.seq, hash, op)))
        .collect();
    ordered.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    ordered.dedup_by(|left, right| left.1 == right.1);
    let mut items: Vec<Item> = Vec::new();
    for (_, _, op) in ordered {
        match op.claim.kind {
            OpKind::Add => {
                let Some(item) = op.claim.item.clone() else {
                    continue;
                };
                match items.iter_mut().find(|held| held.ci == item.ci) {
                    Some(held) => *held = item,
                    None => items.push(item),
                }
            }
            OpKind::Remove => {
                let Some(target) = op.claim.target else {
                    continue;
                };
                items.retain(|held| held.ci != target);
            }
            OpKind::Reorder => {
                let Some(order) = op.claim.order.as_ref() else {
                    continue;
                };
                let mut reordered: Vec<Item> = Vec::with_capacity(items.len());
                for ci in order {
                    if let Some(position) = items.iter().position(|held| &held.ci == ci) {
                        reordered.push(items.remove(position));
                    }
                }
                reordered.append(&mut items);
                items = reordered;
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ci(seed: &[u8]) -> CiHash {
        CiHash::from_hash(BlobHash::of(seed))
    }

    fn owner() -> Identity {
        Identity::from_secret([2; 32])
    }

    #[test]
    fn adds_fold_in_sequence_order_however_they_arrive() {
        let me = owner();
        let first = Op::add(
            "modules/riftbound",
            1,
            Item::labelled(ci(b"v1"), "0.1.0"),
            &me,
        )
        .unwrap();
        let second = Op::add(
            "modules/riftbound",
            2,
            Item::labelled(ci(b"v2"), "0.2.0"),
            &me,
        )
        .unwrap();
        let forward = fold(me.dgid(), &[first.clone(), second.clone()]);
        let backward = fold(me.dgid(), &[second, first]);
        assert_eq!(forward, backward);
        assert_eq!(forward.len(), 2);
        assert_eq!(forward[0].label.as_deref(), Some("0.1.0"));
        assert_eq!(forward[1].label.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn a_repeated_add_replaces_rather_than_duplicates() {
        let me = owner();
        let ops = vec![
            Op::add("modules/mtg", 1, Item::labelled(ci(b"v1"), "0.1.0"), &me).unwrap(),
            Op::add(
                "modules/mtg",
                2,
                Item::labelled(ci(b"v1"), "0.1.0-relabelled"),
                &me,
            )
            .unwrap(),
        ];
        let items = fold(me.dgid(), &ops);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label.as_deref(), Some("0.1.0-relabelled"));
    }

    #[test]
    fn a_remove_drops_the_item_and_a_reorder_moves_it() {
        let me = owner();
        let ops = vec![
            Op::add("cards/mtg", 1, Item::new(ci(b"a")), &me).unwrap(),
            Op::add("cards/mtg", 2, Item::new(ci(b"b")), &me).unwrap(),
            Op::add("cards/mtg", 3, Item::new(ci(b"c")), &me).unwrap(),
            Op::remove("cards/mtg", 4, ci(b"b"), &me).unwrap(),
            Op::reorder("cards/mtg", 5, vec![ci(b"c"), ci(b"a")], &me).unwrap(),
        ];
        let items = fold(me.dgid(), &ops);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].ci, ci(b"c"));
        assert_eq!(items[1].ci, ci(b"a"));
    }

    #[test]
    fn an_op_from_another_key_never_folds() {
        let me = owner();
        let stranger = Identity::from_secret([9; 32]);
        let mine = Op::add("modules/riftbound", 1, Item::new(ci(b"mine")), &me).unwrap();

        let theirs = Op::add("modules/riftbound", 2, Item::new(ci(b"theirs")), &stranger).unwrap();
        let items = fold(me.dgid(), &[mine.clone(), theirs.clone()]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ci, ci(b"mine"));

        let mut forged = theirs;
        forged.claim.owner = me.dgid();
        assert!(!forged.owned());
        assert_eq!(fold(me.dgid(), &[mine, forged]).len(), 1);
    }

    #[test]
    fn an_op_round_trips_and_keeps_its_signature() {
        let me = owner();
        let op = Op::add("modules/mtg", 7, Item::labelled(ci(b"x"), "1.0.0"), &me).unwrap();
        let decoded = Op::decode(&op.encode().unwrap()).unwrap();
        assert_eq!(decoded.signer(), Some(me.dgid()));
        assert_eq!(decoded.address().unwrap(), op.address().unwrap());
    }

    fn scratch(tag: &str) -> BlobStore {
        let dir = std::env::temp_dir().join(format!("spirit-builder-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    #[test]
    fn a_builder_carries_earlier_items_forward_and_replaces_by_identity() {
        let store = scratch("carry");
        let me = owner();
        let mut first = Builder::open(&store, "decks/mtg", &me);
        first.add(Item::labelled(ci(b"a"), "Ashe"));
        first.record(BlobHash::of(b"record a"));
        first.publish(&store, &me).unwrap();

        let mut second = Builder::open(&store, "decks/mtg", &me);
        assert_eq!(second.items().len(), 1);
        second.add(Item::labelled(ci(b"b"), "Jinx"));
        second.add(Item::labelled(ci(b"a"), "Ashe, rebuilt"));
        second.publish(&store, &me).unwrap();

        let third = Builder::open(&store, "decks/mtg", &me);
        assert_eq!(third.items().len(), 2);
        assert_eq!(third.items()[0].label.as_deref(), Some("Ashe, rebuilt"));
        assert_eq!(third.items()[1].label.as_deref(), Some("Jinx"));
        assert!(third.forked_from().is_none());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_builder_over_someone_elses_collection_forks_it() {
        let store = scratch("fork");
        let peer = Identity::from_secret([21; 32]);
        let me = owner();
        let mut theirs = Builder::open(&store, "decks/mtg", &peer);
        theirs.add(Item::labelled(ci(b"theirs"), "their deck"));
        theirs.publish(&store, &peer).unwrap();

        let mut mine = Builder::open(&store, "decks/mtg", &me);
        assert_eq!(mine.items().len(), 1);
        assert_eq!(mine.forked_from(), Some(peer.dgid()));
        mine.add(Item::labelled(ci(b"mine"), "my deck"));
        mine.publish(&store, &me).unwrap();

        let reopened = Builder::open(&store, "decks/mtg", &me);
        assert_eq!(reopened.items().len(), 2);
        assert!(reopened.forked_from().is_none());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_removed_item_stays_gone() {
        let store = scratch("remove");
        let me = owner();
        let mut builder = Builder::open(&store, "decks/mtg", &me);
        builder.add(Item::new(ci(b"a")));
        builder.add(Item::new(ci(b"b")));
        builder.publish(&store, &me).unwrap();

        let mut again = Builder::open(&store, "decks/mtg", &me);
        again.remove(ci(b"a"));
        again.publish(&store, &me).unwrap();

        let reopened = Builder::open(&store, "decks/mtg", &me);
        assert_eq!(reopened.items().len(), 1);
        assert_eq!(reopened.items()[0].ci, ci(b"b"));
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_re_attested_identity_keeps_only_the_newest_claim() {
        let store = scratch("attest");
        let me = owner();
        let mut builder = Builder::open(&store, "decks/mtg", &me);
        let first = BlobHash::of(b"first claim");
        let second = BlobHash::of(b"second claim");
        builder.attest(first);
        builder.attest(second);
        builder.attest(first);
        assert_eq!(builder.attestations(), &[second, first]);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn a_head_lists_every_hash_it_needs_replicated() {
        let me = owner();
        let op = Op::add("modules/mtg", 1, Item::new(ci(b"x")), &me).unwrap();
        let op_hash = op.address().unwrap();
        let record = BlobHash::of(b"the cir");
        let attestation = BlobHash::of(b"the attestation");
        let head = Collection::new("modules", "modules/mtg", me.dgid())
            .with(vec![op_hash], vec![record])
            .attesting(vec![attestation]);
        assert_eq!(head.refs.len(), 3);
        assert!(head.refs.contains(&op_hash));
        assert!(head.refs.contains(&record));
        assert!(head.refs.contains(&attestation));
        let decoded = Collection::decode(&head.encode().unwrap()).unwrap();
        assert_eq!(decoded, head);
        assert!(decoded.address().unwrap().to_string().starts_with("col:"));
    }

    #[test]
    fn a_legacy_head_without_a_record_field_still_decodes_byte_for_byte() {
        let me = owner();
        let legacy = Collection {
            record: String::new(),
            kind: COLLECTION_KIND.into(),
            name: "modules/mtg".into(),
            owner: me.dgid(),
            forked_from: None,
            ops: Vec::new(),
            attestations: Vec::new(),
            records: Vec::new(),
            refs: Vec::new(),
        };
        let bytes = legacy.encode().unwrap();
        let key = [b"\x66".as_slice(), b"record"].concat();
        assert!(!bytes.windows(key.len()).any(|w| w == key));
        let decoded = Collection::decode(&bytes).unwrap();
        assert!(decoded.is_legacy());
        assert_eq!(decoded.encode().unwrap(), bytes);
        let mut wrong = legacy.clone();
        wrong.kind = "playlist".into();
        assert!(Collection::decode(&wrong.encode().unwrap()).is_err());
    }

    #[test]
    fn publishing_appends_ops_instead_of_rewriting_them() {
        let store = scratch("append");
        let me = owner();
        let mut first = Builder::open(&store, "decks/mtg", &me);
        first.kind("playlist");
        first.add(Item::new(ci(b"a")));
        first.add(Item::new(ci(b"b")));
        first.publish(&store, &me).unwrap();
        let (head, _) = load(&store, "decks/mtg").unwrap();
        assert_eq!(head.kind, "playlist");
        assert_eq!(head.record, COLLECTION_RECORD);
        assert_eq!(head.ops.len(), 2);

        let mut second = Builder::open(&store, "decks/mtg", &me);
        assert_eq!(second.pending(), 0);
        second.add(Item::new(ci(b"c")));
        second.remove(ci(b"a"));
        second.publish(&store, &me).unwrap();
        let (again, ops) = load(&store, "decks/mtg").unwrap();
        assert_eq!(again.kind, "playlist");
        assert_eq!(again.ops.len(), 4);
        assert!(head.ops.iter().all(|hash| again.ops.contains(hash)));
        assert_eq!(ops.iter().map(|op| op.claim.seq).max(), Some(4));
        let items = fold(me.dgid(), &ops);
        assert_eq!(
            items.iter().map(|item| item.ci).collect::<Vec<_>>(),
            vec![ci(b"b"), ci(b"c")]
        );

        let mut noop = Builder::open(&store, "decks/mtg", &me);
        noop.add(Item::new(ci(b"d")));
        noop.remove(ci(b"d"));
        noop.publish(&store, &me).unwrap();
        assert_eq!(load(&store, "decks/mtg").unwrap().0.ops.len(), 4);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn concurrent_heads_of_one_owner_merge_by_union() {
        let left = scratch("merge-left");
        let right = scratch("merge-right");
        let me = owner();
        let mut base = Builder::open(&left, "decks/mtg", &me);
        base.add(Item::new(ci(b"a")));
        let base_head = base.publish(&left, &me).unwrap();
        for hash in load(&left, "decks/mtg").unwrap().0.refs {
            right.put(&left.get(hash).unwrap()).unwrap();
        }
        right.put(&left.get(base_head).unwrap()).unwrap();
        assert_eq!(
            merge(&right, "decks/mtg", base_head).unwrap(),
            Merged::Replaced(base_head)
        );

        let mut on_left = Builder::open(&left, "decks/mtg", &me);
        on_left.add(Item::new(ci(b"b")));
        let left_head = on_left.publish(&left, &me).unwrap();
        let mut on_right = Builder::open(&right, "decks/mtg", &me);
        on_right.add(Item::new(ci(b"c")));
        let right_head = on_right.publish(&right, &me).unwrap();

        let left_now = load(&left, "decks/mtg").unwrap().0;
        for hash in left_now.refs.iter().chain([&left_head]) {
            right.put(&left.get(*hash).unwrap()).unwrap();
        }
        let merged = merge(&right, "decks/mtg", left_head).unwrap();
        let Merged::Union(union_head) = merged else {
            panic!("expected a union, got {merged:?}");
        };
        let (head, ops) = load(&right, "decks/mtg").unwrap();
        assert_eq!(crate::refs::read(&right, "decks/mtg"), Some(union_head));
        assert_eq!(head.ops.len(), 3);
        let mut items: Vec<CiHash> = fold(me.dgid(), &ops).into_iter().map(|i| i.ci).collect();
        let mut expected = vec![ci(b"a"), ci(b"b"), ci(b"c")];
        items.sort();
        expected.sort();
        assert_eq!(items, expected);

        assert_eq!(
            merge(&right, "decks/mtg", right_head).unwrap(),
            Merged::Unchanged
        );
        let right_now = load(&right, "decks/mtg").unwrap().0;
        for hash in right_now.refs.iter().chain([&union_head]) {
            left.put(&right.get(*hash).unwrap()).unwrap();
        }
        assert_eq!(
            merge(&left, "decks/mtg", union_head).unwrap(),
            Merged::Adopted(union_head)
        );
        let _ = std::fs::remove_dir_all(left.root());
        let _ = std::fs::remove_dir_all(right.root());
    }

    #[test]
    fn a_different_owner_replaces_rather_than_merges() {
        let store = scratch("merge-owner");
        let me = owner();
        let peer = Identity::from_secret([21; 32]);
        let mut mine = Builder::open(&store, "decks/mtg", &me);
        mine.add(Item::new(ci(b"a")));
        mine.publish(&store, &me).unwrap();
        let theirs = Collection::new("playlist", "decks/mtg", peer.dgid());
        let theirs_hash = store.put(&theirs.encode().unwrap()).unwrap();
        assert_eq!(
            merge(&store, "decks/mtg", theirs_hash).unwrap(),
            Merged::Replaced(theirs_hash)
        );
        let _ = std::fs::remove_dir_all(store.root());
    }
}
