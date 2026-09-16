use ciborium::value::Value;
use serde::{Deserialize, Serialize};
use spirit_core::canonical::CanonError;
use spirit_core::record::Cir;
use spirit_core::{CiHash, Dgid};
use std::collections::BTreeMap;

pub const ITEM_KIND: &str = "item";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external: BTreeMap<String, String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Dgid>,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}

impl Item {
    pub fn new(name: &str) -> Self {
        Self {
            external: BTreeMap::new(),
            name: name.into(),
            owner: None,
            fields: BTreeMap::new(),
        }
    }

    pub fn external(mut self, key: &str, value: &str) -> Self {
        self.external.insert(key.into(), value.into());
        self
    }

    pub fn owned_by(mut self, owner: Dgid) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn field(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn cir(&self) -> Result<Cir, CanonError> {
        Cir::new(ITEM_KIND, self)
    }

    pub fn ci(&self) -> Result<CiHash, CanonError> {
        self.cir()?.address()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_hashes_over_its_identifying_fields_only_and_in_one_order() {
        let song = Item::new("Leaves from the Vine")
            .field("artist", "Uncle Iroh")
            .field("album", "Tales of Ba Sing Se")
            .external("isrc", "US-XYZ-06-00001");
        let same = Item::new("Leaves from the Vine")
            .external("isrc", "US-XYZ-06-00001")
            .field("album", "Tales of Ba Sing Se")
            .field("artist", "Uncle Iroh");
        assert_eq!(song.ci().unwrap(), same.ci().unwrap());
        let cir = song.cir().unwrap();
        assert_eq!(cir.kind, ITEM_KIND);
        let back: Item = Cir::decode(&cir.encode().unwrap()).unwrap().body().unwrap();
        assert_eq!(back, song);
        assert_eq!(back.fields["artist"], Value::Text("Uncle Iroh".into()));
        let owned = song.clone().owned_by(Dgid::from_bytes([7; 32]));
        assert_ne!(owned.ci().unwrap(), song.ci().unwrap());
    }

    #[test]
    fn a_float_field_is_refused_by_the_canonical_encoding() {
        let item = Item::new("x").field("tempo", 1.5f64);
        assert!(item.cir().is_err());
    }
}
