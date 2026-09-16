use crate::address::{AttHash, BlobRef, CiHash, TdHash};
use crate::canonical::{self, CanonError};
use crate::identity::{self, Dgid, Identity, Signature};
use ciborium::value::Value;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const CIR_KIND: &str = "cir";
pub const TDR_KIND: &str = "tdr";
pub const ATTESTATION_KIND: &str = "attestation";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cir {
    pub record: String,
    pub kind: String,
    pub body: Value,
}

impl Cir {
    pub fn new<T: Serialize>(kind: &str, body: &T) -> Result<Self, CanonError> {
        Ok(Self {
            record: CIR_KIND.into(),
            kind: kind.into(),
            body: canonical::from_slice(&canonical::to_vec(body)?)?,
        })
    }

    pub fn body<T: DeserializeOwned>(&self) -> Result<T, CanonError> {
        canonical::from_slice(&canonical::to_vec(&self.body)?)
    }

    pub fn encode(&self) -> Result<Vec<u8>, CanonError> {
        canonical::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CanonError> {
        let record: Self = canonical::from_slice(bytes)?;
        if record.record != CIR_KIND {
            return Err(CanonError::Decode(format!(
                "expected a {CIR_KIND} record, found {:?}",
                record.record
            )));
        }
        Ok(record)
    }

    pub fn address(&self) -> Result<CiHash, CanonError> {
        canonical::hash(self).map(CiHash::from_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tdr {
    pub record: String,
    pub kind: String,
    pub body: Value,
}

impl Tdr {
    pub fn new<T: Serialize>(kind: &str, body: &T) -> Result<Self, CanonError> {
        Ok(Self {
            record: TDR_KIND.into(),
            kind: kind.into(),
            body: canonical::from_slice(&canonical::to_vec(body)?)?,
        })
    }

    pub fn body<T: DeserializeOwned>(&self) -> Result<T, CanonError> {
        canonical::from_slice(&canonical::to_vec(&self.body)?)
    }

    pub fn encode(&self) -> Result<Vec<u8>, CanonError> {
        canonical::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CanonError> {
        let record: Self = canonical::from_slice(bytes)?;
        if record.record != TDR_KIND {
            return Err(CanonError::Decode(format!(
                "expected a {TDR_KIND} record, found {:?}",
                record.record
            )));
        }
        Ok(record)
    }

    pub fn address(&self) -> Result<TdHash, CanonError> {
        canonical::hash(self).map(TdHash::from_hash)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClaimKind {
    Content,
    SameAs,
    SupersededBy,
    PreviousVersion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub kind: ClaimKind,
    pub ci: CiHash,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub td: Option<TdHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<CiHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
}

impl Claim {
    pub fn content(ci: CiHash, td: TdHash, blob: BlobRef) -> Self {
        Self {
            kind: ClaimKind::Content,
            ci,
            td: Some(td),
            blob: Some(blob),
            other: None,
            expires: None,
        }
    }

    pub fn relation(kind: ClaimKind, from: CiHash, to: CiHash) -> Self {
        Self {
            kind,
            ci: from,
            td: None,
            blob: None,
            other: Some(to),
            expires: None,
        }
    }

    pub fn scope(&self) -> Result<Vec<u8>, CanonError> {
        Ok(canonical::hash(self)?.as_bytes().to_vec())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proof {
    pub dgid: Dgid,
    pub sig: Signature,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attestation {
    pub record: String,
    pub claim: Claim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Proof>,
}

impl Attestation {
    pub fn unsigned(claim: Claim) -> Self {
        Self {
            record: ATTESTATION_KIND.into(),
            claim,
            proof: None,
        }
    }

    pub fn sign(claim: Claim, identity: &Identity) -> Result<Self, CanonError> {
        let sig = identity.sign(&claim.scope()?);
        Ok(Self {
            record: ATTESTATION_KIND.into(),
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

    pub fn encode(&self) -> Result<Vec<u8>, CanonError> {
        canonical::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CanonError> {
        let record: Self = canonical::from_slice(bytes)?;
        if record.record != ATTESTATION_KIND {
            return Err(CanonError::Decode(format!(
                "expected an {ATTESTATION_KIND} record, found {:?}",
                record.record
            )));
        }
        Ok(record)
    }

    pub fn address(&self) -> Result<AttHash, CanonError> {
        canonical::hash(self).map(AttHash::from_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::BlobHash;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct CardBody {
        game: String,
        name: String,
    }

    fn bolt() -> Cir {
        Cir::new(
            "card",
            &CardBody {
                game: "mtg".into(),
                name: "Lightning Bolt".into(),
            },
        )
        .unwrap()
    }

    #[test]
    fn a_cir_round_trips_and_keeps_its_address() {
        let cir = bolt();
        let bytes = cir.encode().unwrap();
        let decoded = Cir::decode(&bytes).unwrap();
        assert_eq!(decoded, cir);
        assert_eq!(decoded.address().unwrap(), cir.address().unwrap());
        assert_eq!(
            decoded.body::<CardBody>().unwrap().name,
            "Lightning Bolt".to_string()
        );
        assert!(cir.address().unwrap().to_string().starts_with("ci:"));
    }

    #[test]
    fn two_nodes_minting_the_same_card_mint_the_same_identity() {
        assert_eq!(bolt().address().unwrap(), bolt().address().unwrap());
        let other = Cir::new(
            "card",
            &CardBody {
                game: "mtg".into(),
                name: "Shock".into(),
            },
        )
        .unwrap();
        assert_ne!(bolt().address().unwrap(), other.address().unwrap());
    }

    #[test]
    fn a_record_refuses_to_decode_as_the_wrong_type() {
        let bytes = bolt().encode().unwrap();
        assert!(Tdr::decode(&bytes).is_err());
        assert!(Attestation::decode(&bytes).is_err());
    }

    #[test]
    fn a_signed_attestation_names_its_signer_and_a_tampered_one_does_not() {
        let identity = Identity::from_secret([4; 32]);
        let claim = Claim::content(
            bolt().address().unwrap(),
            TdHash::from_hash(BlobHash::of(b"scryfall image fetch")),
            BlobRef::from_hash(BlobHash::of(b"jpeg bytes")),
        );
        let attestation = Attestation::sign(claim.clone(), &identity).unwrap();
        assert_eq!(attestation.signer(), Some(identity.dgid()));

        let mut forged = attestation.clone();
        forged.claim.blob = Some(BlobRef::from_hash(BlobHash::of(b"other bytes")));
        assert_eq!(forged.signer(), None);

        assert_eq!(Attestation::unsigned(claim).signer(), None);
    }

    #[test]
    fn a_signature_survives_the_record_round_trip() {
        let identity = Identity::from_secret([8; 32]);
        let claim = Claim::relation(
            ClaimKind::SameAs,
            bolt().address().unwrap(),
            CiHash::from_hash(BlobHash::of(b"another mint")),
        );
        let attestation = Attestation::sign(claim, &identity).unwrap();
        let decoded = Attestation::decode(&attestation.encode().unwrap()).unwrap();
        assert_eq!(decoded.signer(), Some(identity.dgid()));
        assert_eq!(decoded.address().unwrap(), attestation.address().unwrap());
    }

    #[test]
    fn wrapping_a_claim_in_a_proof_does_not_move_the_signing_scope() {
        let claim = Claim::content(
            bolt().address().unwrap(),
            TdHash::from_hash(BlobHash::of(b"td")),
            BlobRef::from_hash(BlobHash::of(b"blob")),
        );
        let unsigned = Attestation::unsigned(claim.clone());
        let signed = Attestation::sign(claim.clone(), &Identity::from_secret([1; 32])).unwrap();
        assert_eq!(
            unsigned.claim.scope().unwrap(),
            signed.claim.scope().unwrap()
        );
        assert_eq!(unsigned.claim.scope().unwrap(), claim.scope().unwrap());
    }
}
