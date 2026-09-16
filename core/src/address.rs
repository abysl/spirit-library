use crate::store::BlobHash;
use serde::de::{Deserialize, Deserializer, Error as DeError};
use serde::{Serialize, Serializer};
use std::fmt;

macro_rules! address {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(BlobHash);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn from_hash(hash: BlobHash) -> Self {
                Self(hash)
            }

            pub fn hash(&self) -> BlobHash {
                self.0
            }

            pub fn parse(text: &str) -> Option<Self> {
                BlobHash::parse(text.trim().strip_prefix($prefix)?).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }

        impl From<BlobHash> for $name {
            fn from(hash: BlobHash) -> Self {
                Self(hash)
            }
        }

        impl std::str::FromStr for $name {
            type Err = AddressError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Self::parse(text).ok_or(AddressError {
                    expected: $prefix,
                    found: text.to_string(),
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Self::parse(&text).ok_or_else(|| {
                    D::Error::custom(format!("{:?} is not a {} address", text, $prefix))
                })
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressError {
    pub expected: &'static str,
    pub found: String,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not a {} address", self.found, self.expected)
    }
}

impl std::error::Error for AddressError {}

address!(CiHash, "ci:");
address!(TdHash, "td:");
address!(AttHash, "att:");
address!(ColHash, "col:");
address!(BlobRef, "blob:");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_round_trips_through_its_prefixed_form() {
        let hash = BlobHash::of(b"Lightning Bolt");
        let ci = CiHash::from_hash(hash);
        let text = ci.to_string();
        assert!(text.starts_with("ci:"));
        assert_eq!(text.len(), 3 + 64);
        assert_eq!(CiHash::parse(&text), Some(ci));
        assert_eq!(ci.hash(), hash);
    }

    #[test]
    fn prefixes_do_not_cross() {
        let text = TdHash::from_hash(BlobHash::of(b"scryfall fetch")).to_string();
        assert!(TdHash::parse(&text).is_some());
        assert_eq!(CiHash::parse(&text), None);
        assert_eq!(BlobRef::parse(&text), None);
    }

    #[test]
    fn a_bare_hash_is_not_an_address() {
        let bare = BlobHash::of(b"bytes").to_string();
        assert_eq!(BlobRef::parse(&bare), None);
        assert_eq!(AttHash::parse("att:zz"), None);
        assert_eq!(ColHash::parse(""), None);
    }

    #[test]
    fn addresses_serialize_as_their_prefixed_text() {
        let col = ColHash::from_hash(BlobHash::of(b"modules/riftbound"));
        let bytes = crate::canonical::to_vec(&col).unwrap();
        let decoded: ColHash = crate::canonical::from_slice(&bytes).unwrap();
        assert_eq!(decoded, col);
        let text: String = crate::canonical::from_slice(&bytes).unwrap();
        assert_eq!(text, col.to_string());
    }
}
