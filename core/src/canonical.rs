use crate::store::BlobHash;
use ciborium::value::Value;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonError {
    Encode(String),
    Decode(String),
    Float,
}

impl fmt::Display for CanonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonError::Encode(why) => write!(f, "canonical encode: {why}"),
            CanonError::Decode(why) => write!(f, "canonical decode: {why}"),
            CanonError::Float => {
                write!(
                    f,
                    "canonical encoding rejects floats: no deterministic form"
                )
            }
        }
    }
}

impl std::error::Error for CanonError {}

fn canonicalize(value: Value) -> Result<Value, CanonError> {
    match value {
        Value::Float(_) => Err(CanonError::Float),
        Value::Array(items) => items
            .into_iter()
            .map(canonicalize)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Map(entries) => {
            let mut keyed = Vec::with_capacity(entries.len());
            for (key, item) in entries {
                let key = canonicalize(key)?;
                let item = canonicalize(item)?;
                keyed.push((write(&key)?, key, item));
            }
            keyed.sort_by(|left, right| left.0.cmp(&right.0));
            Ok(Value::Map(
                keyed
                    .into_iter()
                    .map(|(_, key, item)| (key, item))
                    .collect(),
            ))
        }
        Value::Tag(tag, inner) => {
            canonicalize(*inner).map(|inner| Value::Tag(tag, Box::new(inner)))
        }
        other => Ok(other),
    }
}

fn write(value: &Value) -> Result<Vec<u8>, CanonError> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).map_err(|e| CanonError::Encode(e.to_string()))?;
    Ok(bytes)
}

pub fn to_vec<T: Serialize>(value: &T) -> Result<Vec<u8>, CanonError> {
    let value = Value::serialized(value).map_err(|e| CanonError::Encode(e.to_string()))?;
    write(&canonicalize(value)?)
}

pub fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CanonError> {
    ciborium::from_reader(bytes).map_err(|e| CanonError::Decode(e.to_string()))
}

pub fn hash<T: Serialize>(value: &T) -> Result<BlobHash, CanonError> {
    Ok(BlobHash::of(&to_vec(value)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Card {
        name: String,
        set: String,
        number: u32,
    }

    #[derive(Serialize)]
    struct Reordered {
        number: u32,
        set: String,
        name: String,
    }

    #[test]
    fn field_order_does_not_change_the_bytes() {
        let straight = to_vec(&Card {
            name: "Lightning Bolt".into(),
            set: "hob".into(),
            number: 57,
        })
        .unwrap();
        let shuffled = to_vec(&Reordered {
            number: 57,
            set: "hob".into(),
            name: "Lightning Bolt".into(),
        })
        .unwrap();
        assert_eq!(straight, shuffled);
    }

    #[test]
    fn map_keys_sort_bytewise_not_lexically() {
        let mut short_first = BTreeMap::new();
        short_first.insert("zz".to_string(), 1u32);
        short_first.insert("a".to_string(), 2u32);
        let bytes = to_vec(&short_first).unwrap();
        let decoded: Value = ciborium::from_reader(bytes.as_slice()).unwrap();
        let Value::Map(entries) = decoded else {
            panic!("a map encodes as a map");
        };
        assert_eq!(entries[0].0, Value::Text("a".into()));
        assert_eq!(entries[1].0, Value::Text("zz".into()));
    }

    #[test]
    fn floats_are_refused() {
        assert_eq!(to_vec(&1.5f64), Err(CanonError::Float));
        assert_eq!(to_vec(&vec![1.5f32]), Err(CanonError::Float));
    }

    #[test]
    fn integers_take_their_shortest_form() {
        assert_eq!(to_vec(&1u64).unwrap(), vec![0x01]);
        assert_eq!(to_vec(&1u8).unwrap(), vec![0x01]);
        assert_eq!(to_vec(&300u64).unwrap(), vec![0x19, 0x01, 0x2c]);
    }

    #[test]
    fn a_round_trip_returns_the_value() {
        let card = Card {
            name: "Emberwing Scout".into(),
            set: "ogn".into(),
            number: 7,
        };
        let decoded: Card = from_slice(&to_vec(&card).unwrap()).unwrap();
        assert_eq!(decoded, card);
    }

    #[test]
    fn the_same_value_always_hashes_the_same() {
        let card = Card {
            name: "Ember Rune".into(),
            set: "ogn".into(),
            number: 42,
        };
        assert_eq!(hash(&card).unwrap(), hash(&card).unwrap());
        assert_eq!(hash(&card).unwrap(), BlobHash::of(&to_vec(&card).unwrap()));
    }
}
