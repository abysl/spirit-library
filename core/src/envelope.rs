use crate::canonical;
use crate::store::BlobHash;
use ciborium::value::Value;
use serde::{Deserialize, Serialize};

pub const LEGACY_KIND: &str = "legacy";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub kind: String,
    pub refs: Vec<BlobHash>,
}

#[derive(Deserialize)]
struct Declared {
    #[serde(default)]
    record: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    refs: Vec<BlobHash>,
}

pub fn of(bytes: &[u8]) -> Option<Envelope> {
    if let Ok(declared) = canonical::from_slice::<Declared>(bytes) {
        if !declared.refs.is_empty() {
            return Some(Envelope {
                kind: match (declared.record, declared.kind) {
                    (Some(record), Some(kind)) if record != kind => format!("{record} ({kind})"),
                    (Some(record), _) => record,
                    (None, Some(kind)) => kind,
                    (None, None) => LEGACY_KIND.into(),
                },
                refs: declared.refs,
            });
        }
    }
    let value: Value = canonical::from_slice(bytes).ok()?;
    let mut refs = Vec::new();
    scan(&value, &mut refs);
    Some(Envelope {
        kind: LEGACY_KIND.into(),
        refs,
    })
}

fn scan(value: &Value, out: &mut Vec<BlobHash>) {
    match value {
        Value::Text(text) => {
            if let Some(hash) = BlobHash::parse(text) {
                if !out.contains(&hash) {
                    out.push(hash);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| scan(item, out)),
        Value::Map(entries) => entries.iter().for_each(|(key, item)| {
            scan(key, out);
            scan(item, out);
        }),
        Value::Tag(_, inner) => scan(inner, out),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct LegacyCard {
        name: String,
        image: String,
    }

    #[derive(Serialize)]
    struct LegacyManifest {
        set: String,
        cards: Vec<LegacyCard>,
    }

    #[derive(Serialize)]
    struct Declaring {
        kind: String,
        name: String,
        refs: Vec<BlobHash>,
    }

    #[test]
    fn a_declared_envelope_is_taken_at_its_word() {
        let refs = vec![BlobHash::of(b"one"), BlobHash::of(b"two")];
        let bytes = canonical::to_vec(&Declaring {
            kind: "collection".into(),
            name: "modules/mtg".into(),
            refs: refs.clone(),
        })
        .unwrap();
        let envelope = of(&bytes).unwrap();
        assert_eq!(envelope.kind, "collection");
        assert_eq!(envelope.refs, refs);
    }

    #[test]
    fn a_legacy_card_manifest_still_yields_its_blobs() {
        let art = BlobHash::of(b"jpeg");
        let other = BlobHash::of(b"another jpeg");
        let bytes = canonical::to_vec(&LegacyManifest {
            set: "hob".into(),
            cards: vec![
                LegacyCard {
                    name: "Lightning Bolt".into(),
                    image: art.to_string(),
                },
                LegacyCard {
                    name: "Shock".into(),
                    image: other.to_string(),
                },
            ],
        })
        .unwrap();
        let envelope = of(&bytes).unwrap();
        assert_eq!(envelope.kind, LEGACY_KIND);
        assert_eq!(envelope.refs.len(), 2);
        assert!(envelope.refs.contains(&art));
        assert!(envelope.refs.contains(&other));
    }

    #[test]
    fn a_repeated_hash_counts_once() {
        let art = BlobHash::of(b"shared art");
        let bytes = canonical::to_vec(&LegacyManifest {
            set: "hob".into(),
            cards: vec![
                LegacyCard {
                    name: "Front".into(),
                    image: art.to_string(),
                },
                LegacyCard {
                    name: "Back".into(),
                    image: art.to_string(),
                },
            ],
        })
        .unwrap();
        assert_eq!(of(&bytes).unwrap().refs, vec![art]);
    }

    #[test]
    fn a_record_with_no_hashes_has_an_empty_envelope() {
        let bytes = canonical::to_vec(&LegacyManifest {
            set: "empty".into(),
            cards: Vec::new(),
        })
        .unwrap();
        assert!(of(&bytes).unwrap().refs.is_empty());
        assert!(of(&[0xff, 0xff]).is_none());
        assert!(of(&[]).is_none());
    }
}
