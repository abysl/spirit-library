use crate::identity::Dgid;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const TRUST_FILE: &str = "trust";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    Unknown,
    Contact,
    Cache,
    Mesh,
}

impl TrustLevel {
    pub fn label(&self) -> &'static str {
        match self {
            TrustLevel::Unknown => "unknown",
            TrustLevel::Contact => "contact",
            TrustLevel::Cache => "cache",
            TrustLevel::Mesh => "mesh",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "unknown" => Some(TrustLevel::Unknown),
            "contact" => Some(TrustLevel::Contact),
            "cache" => Some(TrustLevel::Cache),
            "mesh" => Some(TrustLevel::Mesh),
            _ => None,
        }
    }
}

impl fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Trust {
    own: Option<Dgid>,
    levels: BTreeMap<Dgid, TrustLevel>,
}

impl Trust {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_own(mut self, dgid: Dgid) -> Self {
        self.own = Some(dgid);
        self
    }

    pub fn set_own(&mut self, dgid: Dgid) {
        self.own = Some(dgid);
    }

    pub fn own(&self) -> Option<Dgid> {
        self.own
    }

    pub fn path(dir: &Path) -> PathBuf {
        dir.join(TRUST_FILE)
    }

    pub fn load(dir: &Path) -> Self {
        let mut trust = Self::new();
        let Ok(text) = std::fs::read_to_string(Self::path(dir)) else {
            return trust;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((dgid, level)) = line.split_once(char::is_whitespace) else {
                continue;
            };
            if let (Some(dgid), Some(level)) = (Dgid::parse(dgid), TrustLevel::parse(level)) {
                trust.levels.insert(dgid, level);
            }
        }
        trust
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text: String = self
            .levels
            .iter()
            .map(|(dgid, level)| format!("{dgid} {level}\n"))
            .collect();
        std::fs::write(Self::path(dir), text)
    }

    pub fn set(&mut self, dgid: Dgid, level: TrustLevel) {
        if level == TrustLevel::Unknown {
            self.levels.remove(&dgid);
        } else {
            self.levels.insert(dgid, level);
        }
    }

    pub fn level(&self, dgid: Dgid) -> TrustLevel {
        if self.own == Some(dgid) {
            return TrustLevel::Mesh;
        }
        self.levels
            .get(&dgid)
            .copied()
            .unwrap_or(TrustLevel::Unknown)
    }

    pub fn trusts(&self, dgid: Dgid, at_least: TrustLevel) -> bool {
        self.level(dgid) >= at_least
    }

    pub fn entries(&self) -> impl Iterator<Item = (&Dgid, &TrustLevel)> {
        self.levels.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn dgid(seed: u8) -> Dgid {
        Identity::from_secret([seed; 32]).dgid()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spirit-trust-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn levels_order_from_unknown_up_to_mesh() {
        assert!(TrustLevel::Mesh > TrustLevel::Cache);
        assert!(TrustLevel::Cache > TrustLevel::Contact);
        assert!(TrustLevel::Contact > TrustLevel::Unknown);
    }

    #[test]
    fn an_unlisted_key_is_unknown_and_my_own_key_is_mesh() {
        let mine = dgid(1);
        let stranger = dgid(2);
        let trust = Trust::new().with_own(mine);
        assert_eq!(trust.level(mine), TrustLevel::Mesh);
        assert_eq!(trust.level(stranger), TrustLevel::Unknown);
        assert!(trust.trusts(mine, TrustLevel::Cache));
        assert!(!trust.trusts(stranger, TrustLevel::Contact));
    }

    #[test]
    fn levels_survive_a_save_and_load() {
        let dir = scratch("roundtrip");
        let mut trust = Trust::new();
        trust.set(dgid(3), TrustLevel::Cache);
        trust.set(dgid(4), TrustLevel::Contact);
        trust.save(&dir).unwrap();

        let loaded = Trust::load(&dir).with_own(dgid(5));
        assert_eq!(loaded.level(dgid(3)), TrustLevel::Cache);
        assert_eq!(loaded.level(dgid(4)), TrustLevel::Contact);
        assert_eq!(loaded.level(dgid(5)), TrustLevel::Mesh);
        assert_eq!(loaded.entries().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_unknown_forgets_a_key() {
        let mut trust = Trust::new();
        trust.set(dgid(6), TrustLevel::Mesh);
        trust.set(dgid(6), TrustLevel::Unknown);
        assert_eq!(trust.level(dgid(6)), TrustLevel::Unknown);
        assert_eq!(trust.entries().count(), 0);
    }

    #[test]
    fn a_missing_trust_file_loads_as_empty() {
        let dir = scratch("missing");
        assert_eq!(Trust::load(&dir).entries().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
