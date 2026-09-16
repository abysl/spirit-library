pub mod transform;

use spirit_core::record::{Attestation, ClaimKind, Tdr};
use spirit_core::{clock, BlobHash, BlobStore, CiHash, Dgid, TdHash, Trust, TrustLevel};
use spirit_index::Index;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub minimum: TrustLevel,
    pub prefer_held: bool,
    pub prefer_td: Option<TdHash>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            minimum: TrustLevel::Cache,
            prefer_held: true,
            prefer_td: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub blob: BlobHash,
    pub signer: Dgid,
    pub td: Option<TdHash>,
    pub held: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub blob: BlobHash,
    pub td: Option<TdHash>,
    pub td_kind: Option<String>,
    pub variant: Option<String>,
    pub snapshot: Option<String>,
    pub signer: Option<Dgid>,
    pub level: TrustLevel,
    pub held: bool,
    pub expires: Option<String>,
    pub expired: bool,
    pub eligible: bool,
}

impl Candidate {
    pub fn resolution(&self) -> Option<Resolution> {
        Some(Resolution {
            blob: self.blob,
            signer: self.signer?,
            td: self.td,
            held: self.held,
        })
    }

    fn rank(&self, policy: Policy) -> (bool, bool, bool, TrustLevel, String, BlobHash) {
        (
            self.eligible,
            policy.prefer_td.is_some() && policy.prefer_td == self.td,
            policy.prefer_held && self.held,
            self.level,
            self.snapshot.clone().unwrap_or_default(),
            self.blob,
        )
    }
}

fn text_field(tdr: &Tdr, name: &str) -> Option<String> {
    let ciborium::value::Value::Map(entries) = &tdr.body else {
        return None;
    };
    entries.iter().find_map(|(key, item)| match (key, item) {
        (ciborium::value::Value::Text(key), ciborium::value::Value::Text(text)) if key == name => {
            Some(text.clone())
        }
        _ => None,
    })
}

fn candidate(
    store: &BlobStore,
    trust: &Trust,
    policy: Policy,
    now: &str,
    attestation: &Attestation,
) -> Option<Candidate> {
    if attestation.claim.kind != ClaimKind::Content {
        return None;
    }
    let blob = attestation.claim.blob?.hash();
    let signer = attestation.signer();
    let level = signer
        .map(|dgid| trust.level(dgid))
        .unwrap_or(TrustLevel::Unknown);
    let tdr = attestation
        .claim
        .td
        .and_then(|td| store.get(td.hash()).ok())
        .and_then(|bytes| Tdr::decode(&bytes).ok());
    let expired = clock::expired(attestation.claim.expires.as_deref(), now);
    Some(Candidate {
        blob,
        td: attestation.claim.td,
        td_kind: tdr.as_ref().map(|tdr| tdr.kind.clone()),
        variant: tdr.as_ref().and_then(|tdr| text_field(tdr, "variant")),
        snapshot: tdr.as_ref().and_then(|tdr| text_field(tdr, "snapshot")),
        signer,
        level,
        held: store.has(blob),
        expires: attestation.claim.expires.clone(),
        expired,
        eligible: signer.is_some() && level >= policy.minimum && !expired,
    })
}

pub fn artifacts(
    store: &BlobStore,
    index: &Index,
    trust: &Trust,
    ci: CiHash,
    policy: Policy,
) -> Vec<Candidate> {
    let now = clock::now_rfc3339();
    let mut found: Vec<Candidate> = index
        .attestations_for(ci)
        .iter()
        .filter_map(|attestation| candidate(store, trust, policy, &now, attestation))
        .collect();
    found.sort_by_key(|candidate| std::cmp::Reverse(candidate.rank(policy)));
    found
}

pub fn resolve(
    store: &BlobStore,
    index: &Index,
    trust: &Trust,
    ci: CiHash,
    policy: Policy,
) -> Option<Resolution> {
    artifacts(store, index, trust, ci, policy)
        .into_iter()
        .find(|candidate| candidate.eligible)
        .and_then(|candidate| candidate.resolution())
}

pub fn rank_providers<T: Clone>(
    trust: &Trust,
    policy: Policy,
    providers: impl IntoIterator<Item = (Dgid, T)>,
) -> Vec<T> {
    let mut ranked: Vec<(TrustLevel, T)> = providers
        .into_iter()
        .map(|(dgid, item)| (trust.level(dgid), item))
        .filter(|(level, _)| *level >= policy.minimum)
        .collect();
    ranked.sort_by_key(|(level, _)| std::cmp::Reverse(*level));
    ranked.into_iter().map(|(_, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use spirit_core::collection::{self, Collection};
    use spirit_core::record::{Claim, Tdr};
    use spirit_core::{BlobRef, Identity};

    fn scratch(tag: &str) -> BlobStore {
        let dir = std::env::temp_dir().join(format!("spirit-routing-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(dir).unwrap()
    }

    fn attest(store: &BlobStore, identity: &Identity, ci: CiHash, blob: BlobHash) -> BlobHash {
        attest_via(store, identity, ci, blob, "fetch", None, None).0
    }

    fn attest_via(
        store: &BlobStore,
        identity: &Identity,
        ci: CiHash,
        blob: BlobHash,
        variant: &str,
        snapshot: Option<&str>,
        expires: Option<&str>,
    ) -> (BlobHash, TdHash) {
        let mut body = serde_json::json!({ "variant": variant });
        if let Some(snapshot) = snapshot {
            body["snapshot"] = serde_json::Value::String(snapshot.into());
        }
        let td = TdHash::from_hash(
            store
                .put(&Tdr::new("fetch", &body).unwrap().encode().unwrap())
                .unwrap(),
        );
        let mut claim = Claim::content(ci, td, BlobRef::from_hash(blob));
        claim.expires = expires.map(String::from);
        let attestation = Attestation::sign(claim, identity).unwrap();
        (store.put(&attestation.encode().unwrap()).unwrap(), td)
    }

    fn ci_of(store: &BlobStore, name: &str) -> CiHash {
        let cir = spirit_core::record::Cir::new("item", &(name,)).unwrap();
        CiHash::from_hash(store.put(&cir.encode().unwrap()).unwrap())
    }

    #[test]
    fn the_higher_trusted_attestation_wins() {
        let store = scratch("trust");
        let me = Identity::from_secret([1; 32]);
        let friend = Identity::from_secret([2; 32]);
        let ci = ci_of(&store, "Lightning Bolt");
        let mine = store.put(b"my scan").unwrap();
        let theirs = store.put(b"their scan").unwrap();
        let records = vec![
            ci.hash(),
            attest(&store, &me, ci, mine),
            attest(&store, &friend, ci, theirs),
        ];
        collection::publish(
            &store,
            &Collection::new("catalog", "cards/mtg", me.dgid()).with(Vec::new(), records),
        )
        .unwrap();

        let index = Index::build(&store);
        let mut trust = Trust::new().with_own(me.dgid());
        trust.set(friend.dgid(), TrustLevel::Cache);
        let resolved = resolve(&store, &index, &trust, ci, Policy::default()).unwrap();
        assert_eq!(resolved.signer, me.dgid());
        assert_eq!(resolved.blob, mine);
        assert!(resolved.held);
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn an_untrusted_claim_resolves_to_nothing() {
        let store = scratch("untrusted");
        let stranger = Identity::from_secret([3; 32]);
        let ci = ci_of(&store, "Shock");
        let blob = store.put(b"bytes").unwrap();
        collection::publish(
            &store,
            &Collection::new("catalog", "cards/mtg", stranger.dgid()).with(
                Vec::new(),
                vec![ci.hash(), attest(&store, &stranger, ci, blob)],
            ),
        )
        .unwrap();

        let index = Index::build(&store);
        let trust = Trust::new().with_own(Identity::from_secret([4; 32]).dgid());
        assert!(resolve(&store, &index, &trust, ci, Policy::default()).is_none());

        let open = Policy {
            minimum: TrustLevel::Unknown,
            ..Policy::default()
        };
        assert!(resolve(&store, &index, &trust, ci, open).is_some());
        let _ = std::fs::remove_dir_all(store.root());
    }

    #[test]
    fn providers_rank_by_trust_and_drop_the_untrusted() {
        let mine = Identity::from_secret([5; 32]).dgid();
        let cache = Identity::from_secret([6; 32]).dgid();
        let contact = Identity::from_secret([7; 32]).dgid();
        let stranger = Identity::from_secret([8; 32]).dgid();
        let mut trust = Trust::new().with_own(mine);
        trust.set(cache, TrustLevel::Cache);
        trust.set(contact, TrustLevel::Contact);

        let policy = Policy {
            minimum: TrustLevel::Contact,
            prefer_held: false,
            prefer_td: None,
        };
        let ranked = rank_providers(
            &trust,
            policy,
            [
                (contact, "contact"),
                (stranger, "stranger"),
                (mine, "mine"),
                (cache, "cache"),
            ],
        );
        assert_eq!(ranked, vec!["mine", "cache", "contact"]);
    }

    #[test]
    fn the_preferred_transform_then_the_newest_snapshot_wins_and_expired_claims_lose() {
        let store = scratch("ranking");
        let me = Identity::from_secret([1; 32]);
        let ci = ci_of(&store, "Leaves from the Vine");
        let flac = store.put(b"flac").unwrap();
        let mp3 = store.put(b"mp3").unwrap();
        let remaster = store.put(b"remaster").unwrap();
        let stale = store.put(b"stale").unwrap();
        let (a_flac, td_flac) = attest_via(
            &store,
            &me,
            ci,
            flac,
            "flac",
            Some("2026-01-01T00:00:00Z"),
            None,
        );
        let (a_mp3, _) = attest_via(
            &store,
            &me,
            ci,
            mp3,
            "mp3",
            Some("2026-02-01T00:00:00Z"),
            None,
        );
        let (a_remaster, _) = attest_via(
            &store,
            &me,
            ci,
            remaster,
            "flac",
            Some("2026-03-01T00:00:00Z"),
            None,
        );
        let (a_stale, _) = attest_via(
            &store,
            &me,
            ci,
            stale,
            "flac",
            Some("2027-01-01T00:00:00Z"),
            Some("2000-01-01T00:00:00Z"),
        );
        collection::publish(
            &store,
            &Collection::new("playlist", "songs", me.dgid()).with(
                Vec::new(),
                vec![ci.hash(), a_flac, a_mp3, a_remaster, a_stale],
            ),
        )
        .unwrap();
        let index = Index::build(&store);
        let trust = Trust::new().with_own(me.dgid());

        let newest = resolve(&store, &index, &trust, ci, Policy::default()).unwrap();
        assert_eq!(newest.blob, remaster);

        let preferred = Policy {
            prefer_td: Some(td_flac),
            ..Policy::default()
        };
        assert_eq!(
            resolve(&store, &index, &trust, ci, preferred).unwrap().blob,
            flac
        );

        let listed = artifacts(&store, &index, &trust, ci, Policy::default());
        assert_eq!(listed.len(), 4);
        let expired: Vec<&Candidate> = listed.iter().filter(|c| c.expired).collect();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].blob, stale);
        assert!(!expired[0].eligible);
        assert_eq!(listed[0].variant.as_deref(), Some("flac"));
        assert_eq!(listed[0].snapshot.as_deref(), Some("2026-03-01T00:00:00Z"));
        let _ = std::fs::remove_dir_all(store.root());
    }
}
