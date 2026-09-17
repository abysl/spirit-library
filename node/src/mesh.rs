use crate::gossip::{self, RefAdvert, TableAdvert, View, ViewSource};
#[cfg(feature = "native")]
use crate::iroh_hash;
use crate::peers::{self, RefStatus};
use crate::tables::TableBook;
use iroh::{Endpoint, EndpointAddr, EndpointId};
#[cfg(feature = "native")]
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::ticket::BlobTicket;
use iroh_tickets::endpoint::EndpointTicket;
use n0_future::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use spirit_core::collection;
use spirit_core::{envelope, refs, BlobHash, BlobStore, Dgid, Trust, TrustLevel};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const ROUND_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_RECHECK: Duration = Duration::from_secs(60);
const FANOUT: usize = 4;
const DIAL_FAILURE_LIMIT: u32 = 5;
const BACKFILL_SETTLE: Duration = Duration::from_secs(30);
const BACKFILL_RETRY: Duration = Duration::from_secs(600);

pub type Backfill = Arc<dyn Fn(&str, &Path) -> Result<(), String> + Send + Sync>;
pub type Fetch = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

#[derive(Debug, Clone)]
struct GossipMark {
    version: u64,
    at: Instant,
    failures: u32,
}

pub struct Mesh {
    store_dir: PathBuf,
    endpoint: Endpoint,
    self_id: String,
    known: Mutex<BTreeMap<String, EndpointAddr>>,
    peer_refs: Mutex<BTreeMap<String, Vec<RefAdvert>>>,
    wanted: Mutex<BTreeSet<String>>,
    version: AtomicU64,
    marks: Mutex<BTreeMap<String, GossipMark>>,
    started: Instant,
    replicate: AtomicBool,
    published_only: AtomicBool,
    backfill: Mutex<Option<Backfill>>,
    backfilled: Mutex<BTreeMap<String, Instant>>,
    table: Mutex<Option<TableAdvert>>,
    tables: Mutex<TableBook>,
    fetch: Mutex<Option<Fetch>>,
    blob_wants: Mutex<BTreeMap<String, Option<String>>>,
    served_blobs: Mutex<BTreeSet<String>>,
    seeded: Mutex<BTreeSet<String>>,
    forgotten: Mutex<BTreeSet<String>>,
    trust: Mutex<Trust>,
    group: Mutex<Option<(Dgid, String)>>,
    peer_dgids: Mutex<BTreeMap<String, Dgid>>,
    members: Mutex<BTreeSet<String>>,
    #[cfg(feature = "native")]
    offer: Mutex<Option<crate::pair::Offer>>,
    merged_heads: Mutex<BTreeSet<String>>,
    #[cfg(feature = "native")]
    lock: Mutex<Option<crate::lock::StoreLock>>,
}

pub const PEERS_FILE: &str = "peers";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberedPeer {
    pub id: String,
    pub addr: EndpointAddr,
    #[serde(default)]
    pub first_seen: u64,
    #[serde(default)]
    pub last_activity: u64,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RememberedPeers {
    #[serde(default)]
    pub peers: Vec<RememberedPeer>,
    #[serde(default)]
    pub forgotten: Vec<String>,
}

fn epoch_secs(time: n0_future::time::SystemTime) -> u64 {
    time.duration_since(n0_future::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn from_epoch(secs: u64) -> n0_future::time::SystemTime {
    n0_future::time::SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

pub fn wall_clock() -> u64 {
    use n0_future::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn read_remembered(dir: &Path) -> RememberedPeers {
    std::fs::read_to_string(dir.join(PEERS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn admits_introduction(
    forgotten: &BTreeSet<String>,
    id: &str,
    introduced_by: Option<&str>,
) -> bool {
    introduced_by.is_none() || !forgotten.contains(id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTable {
    pub host: String,
    pub name: String,
    pub relayed: bool,
}

pub fn gateway_url(value: &str) -> Option<String> {
    let value = value.trim();
    if !(value.starts_with("http://") || value.starts_with("https://")) {
        return None;
    }
    Some(value.trim_end_matches('/').to_string())
}

pub fn status_url(base: &str) -> String {
    format!("{}/gateway/status", base.trim_end_matches('/'))
}

pub fn seed_from_status(json: &str) -> Result<String, Box<dyn Error>> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let ticket = value
        .get("ticket")
        .and_then(|v| v.as_str())
        .filter(|t| t.parse::<EndpointTicket>().is_ok());
    let node_id = value
        .get("node_id")
        .and_then(|v| v.as_str())
        .filter(|id| id.parse::<EndpointId>().is_ok());
    ticket
        .or(node_id)
        .map(String::from)
        .ok_or_else(|| "the gateway status names no usable ticket or node id".into())
}

pub fn is_seed(value: &str) -> Result<(), Box<dyn Error>> {
    if gateway_url(value).is_some() {
        return Ok(());
    }
    parse_seed(value).map(|_| ())
}

pub fn parse_seed(value: &str) -> Result<EndpointAddr, Box<dyn Error>> {
    let value = value.trim();
    if let Ok(parsed) = value.parse::<EndpointTicket>() {
        return Ok(parsed.endpoint_addr().clone());
    }
    if let Ok(parsed) = value.parse::<BlobTicket>() {
        return Ok(parsed.addr().clone());
    }
    Ok(EndpointAddr::new(value.parse::<EndpointId>()?))
}

impl std::fmt::Debug for Mesh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mesh")
            .field("self_id", &self.self_id)
            .field("known", &self.known.lock().unwrap().len())
            .field("version", &self.version.load(Ordering::Relaxed))
            .finish()
    }
}

pub fn safe_ref_name(name: &str) -> bool {
    refs::valid_name(name)
}

pub fn dgid_of(id: &str) -> Option<Dgid> {
    id.parse::<EndpointId>()
        .ok()
        .map(|key| Dgid::from_bytes(*key.as_bytes()))
}

pub fn local_refs(dir: &Path) -> Vec<RefAdvert> {
    let Ok(store) = BlobStore::open(dir) else {
        return Vec::new();
    };
    refs::list(&store)
        .into_iter()
        .map(|(name, manifest)| {
            let (total, held) = completeness(&store, manifest);
            let owner = store
                .get(manifest)
                .ok()
                .and_then(|bytes| collection::Collection::decode(&bytes).ok())
                .map(|head| head.owner.to_string());
            RefAdvert {
                name,
                manifest: manifest.to_string(),
                total,
                held,
                owner,
            }
        })
        .collect()
}

fn referenced(store: &BlobStore, manifest: BlobHash) -> Vec<BlobHash> {
    let Ok(bytes) = store.get(manifest) else {
        return Vec::new();
    };
    envelope::of(&bytes)
        .map(|envelope| envelope.refs)
        .unwrap_or_default()
}

fn completeness(store: &BlobStore, manifest: BlobHash) -> (u32, u32) {
    let refs = referenced(store, manifest);
    let held = refs.iter().filter(|hash| store.has(**hash)).count();
    (refs.len() as u32, held as u32)
}

impl Mesh {
    fn load_group(store_dir: &Path, node_id: &[u8; 32]) -> Option<(Dgid, String)> {
        if !store_dir.is_dir() {
            return None;
        }
        let group = spirit_core::identity::load_or_create(store_dir).ok()?;
        let vouch = spirit_core::identity::vouch(&group, node_id).to_string();
        Some((group.dgid(), vouch))
    }

    pub fn new(store_dir: &Path, endpoint: Endpoint) -> Arc<Self> {
        let mut trust = Trust::load(store_dir);
        let node_id = endpoint.id();
        let group = Self::load_group(store_dir, node_id.as_bytes());
        match group.as_ref().map(|(dgid, _)| *dgid) {
            Some(own) => trust = trust.with_own(own),
            None => {
                if let Some(own) = dgid_of(&node_id.to_string()) {
                    trust = trust.with_own(own);
                }
            }
        }
        let mesh = Arc::new(Self {
            store_dir: store_dir.to_path_buf(),
            self_id: endpoint.id().to_string(),
            endpoint,
            known: Mutex::new(BTreeMap::new()),
            peer_refs: Mutex::new(BTreeMap::new()),
            wanted: Mutex::new(BTreeSet::new()),
            version: AtomicU64::new(1),
            marks: Mutex::new(BTreeMap::new()),
            started: Instant::now(),
            replicate: AtomicBool::new(true),
            published_only: AtomicBool::new(false),
            backfill: Mutex::new(None),
            backfilled: Mutex::new(BTreeMap::new()),
            table: Mutex::new(None),
            tables: Mutex::new(TableBook::default()),
            fetch: Mutex::new(None),
            blob_wants: Mutex::new(BTreeMap::new()),
            served_blobs: Mutex::new(BTreeSet::new()),
            seeded: Mutex::new(BTreeSet::new()),
            forgotten: Mutex::new(BTreeSet::new()),
            trust: Mutex::new(trust),
            group: Mutex::new(group),
            peer_dgids: Mutex::new(BTreeMap::new()),
            members: Mutex::new(BTreeSet::new()),
            #[cfg(feature = "native")]
            offer: Mutex::new(None),
            merged_heads: Mutex::new(BTreeSet::new()),
            #[cfg(feature = "native")]
            lock: Mutex::new(None),
        });
        mesh.refresh_membership();
        mesh.restore_peers();
        mesh
    }

    #[cfg(feature = "native")]
    pub fn set_lock(&self, lock: crate::lock::StoreLock) {
        *self.lock.lock().unwrap() = Some(lock);
    }

    #[cfg(feature = "native")]
    pub fn heartbeat(&self) {
        if let Some(lock) = self.lock.lock().unwrap().as_ref() {
            lock.touch();
        }
    }

    fn restore_peers(&self) {
        if !self.store_dir.is_dir() {
            return;
        }
        let remembered = read_remembered(&self.store_dir);
        let mut known = self.known.lock().unwrap();
        for peer in remembered.peers {
            if peer.id == self.self_id || known.contains_key(&peer.id) {
                continue;
            }
            known.insert(peer.id.clone(), peer.addr);
            peers::registry().remember(
                &peer.id,
                from_epoch(peer.first_seen),
                from_epoch(peer.last_activity),
                peer.refs,
                peer.ticket,
            );
        }
        self.forgotten.lock().unwrap().extend(remembered.forgotten);
    }

    pub fn persist_peers(&self) {
        if !self.store_dir.is_dir() {
            return;
        }
        let known = self.known.lock().unwrap();
        let snapshots: BTreeMap<String, peers::PeerSnapshot> = peers::snapshot()
            .into_iter()
            .map(|peer| (peer.id.clone(), peer))
            .collect();
        let remembered = RememberedPeers {
            peers: known
                .iter()
                .map(|(id, addr)| {
                    let seen = snapshots.get(id);
                    RememberedPeer {
                        id: id.clone(),
                        addr: addr.clone(),
                        first_seen: seen.map(|p| epoch_secs(p.first_seen)).unwrap_or(0),
                        last_activity: seen.map(|p| epoch_secs(p.last_activity)).unwrap_or(0),
                        refs: seen.map(|p| p.refs_advertised.clone()).unwrap_or_default(),
                        ticket: seen.and_then(|p| p.ticket.clone()),
                    }
                })
                .collect(),
            forgotten: self.forgotten.lock().unwrap().iter().cloned().collect(),
        };
        drop(known);
        if let Ok(text) = serde_json::to_string_pretty(&remembered) {
            let path = self.store_dir.join(PEERS_FILE);
            let temp = self
                .store_dir
                .join(format!("{PEERS_FILE}.tmp-{}", std::process::id()));
            if std::fs::write(&temp, text).is_ok() {
                let _ = std::fs::rename(&temp, &path);
            }
        }
    }

    pub fn reload_identity(&self) {
        let node_id = self.endpoint.id();
        let group = Self::load_group(&self.store_dir, node_id.as_bytes());
        let mut trust = Trust::load(&self.store_dir);
        match group.as_ref().map(|(dgid, _)| *dgid) {
            Some(own) => trust.set_own(own),
            None => {
                if let Some(own) = dgid_of(&node_id.to_string()) {
                    trust.set_own(own);
                }
            }
        }
        *self.group.lock().unwrap() = group;
        *self.trust.lock().unwrap() = trust;
        self.version.fetch_add(1, Ordering::Relaxed);
        self.refresh_membership();
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    fn group_dgid(&self) -> Option<Dgid> {
        self.group.lock().unwrap().as_ref().map(|(dgid, _)| *dgid)
    }

    pub fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub fn endpoint_addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    pub fn refresh_membership(&self) {
        let Some(group) = self.group_dgid() else {
            return;
        };
        let Ok(store) = BlobStore::open(&self.store_dir) else {
            return;
        };
        let ids: BTreeSet<String> = spirit_schema::device::member_ids(&store, group)
            .into_iter()
            .collect();
        let mut members = self.members.lock().unwrap();
        if *members != ids {
            *members = ids;
            self.version.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn members(&self) -> Vec<String> {
        self.members.lock().unwrap().iter().cloned().collect()
    }

    pub fn is_member(&self, id: &str) -> bool {
        self.members.lock().unwrap().contains(id)
    }

    #[cfg(feature = "native")]
    pub fn offer_pairing(&self) -> Option<crate::pair::Invite> {
        let dgid = self.group_dgid()?;
        let offer = crate::pair::Offer::new().ok()?;
        let invite = crate::pair::Invite {
            ticket: EndpointTicket::from(self.endpoint.addr()).to_string(),
            dgid,
            token: offer.token,
        };
        *self.offer.lock().unwrap() = Some(offer);
        Some(invite)
    }

    #[cfg(feature = "native")]
    pub fn current_offer(&self) -> Option<crate::pair::Offer> {
        self.offer
            .lock()
            .unwrap()
            .clone()
            .filter(|offer| offer.live())
    }

    #[cfg(feature = "native")]
    pub fn take_offer(&self, token: &[u8; 32]) -> bool {
        let mut slot = self.offer.lock().unwrap();
        let matches = slot.as_ref().is_some_and(|offer| {
            offer.live() && crate::pair::constant_time_eq(&offer.token, token)
        });
        if matches {
            *slot = None;
        }
        matches
    }

    pub fn set_backfill(&self, backfill: Backfill) {
        *self.backfill.lock().unwrap() = Some(backfill);
    }

    pub fn set_fetch(&self, fetch: Fetch) {
        *self.fetch.lock().unwrap() = Some(fetch);
    }

    fn fetch_text(&self, url: &str) -> Result<String, Box<dyn Error>> {
        let hook = self.fetch.lock().unwrap().clone();
        let body = match hook {
            Some(fetch) => fetch(url)?,
            None => Self::builtin_fetch(url)?,
        };
        Ok(body)
    }

    #[cfg(feature = "native")]
    fn builtin_fetch(url: &str) -> Result<String, String> {
        crate::fetch::get(url)
    }

    #[cfg(not(feature = "native"))]
    fn builtin_fetch(_url: &str) -> Result<String, String> {
        Err("gateway url seeds need a fetcher from the caller (Mesh::set_fetch)".into())
    }

    fn resolve_gateway(&self, base: &str) -> Result<String, Box<dyn Error>> {
        let url = status_url(base);
        let body = self
            .fetch_text(&url)
            .map_err(|error| format!("fetching {url}: {error}"))?;
        seed_from_status(&body).map_err(|error| format!("{url}: {error}").into())
    }

    pub fn set_replicate(&self, replicate: bool) {
        self.replicate.store(
            replicate && !self.published_only.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }

    pub fn publish_only(&self) {
        self.published_only.store(true, Ordering::Relaxed);
        self.set_replicate(false);
    }

    pub fn accepts_downloads(&self) -> bool {
        !self.published_only.load(Ordering::Relaxed)
    }

    pub fn replicates(&self) -> bool {
        self.replicate.load(Ordering::Relaxed)
    }

    pub fn self_id(&self) -> &str {
        &self.self_id
    }

    pub fn self_dgid(&self) -> Option<Dgid> {
        self.group_dgid().or_else(|| dgid_of(&self.self_id))
    }

    pub fn dgid_of_peer(&self, id: &str) -> Option<Dgid> {
        self.peer_dgids.lock().unwrap().get(id).copied()
    }

    pub fn trust_level(&self, id: &str) -> TrustLevel {
        if id == self.self_id || self.is_member(id) {
            return TrustLevel::Mesh;
        }
        let trust = self.trust.lock().unwrap();
        let by_device = dgid_of(id)
            .map(|dgid| trust.level(dgid))
            .unwrap_or(TrustLevel::Unknown);
        let by_group = self
            .peer_dgids
            .lock()
            .unwrap()
            .get(id)
            .map(|dgid| trust.level(*dgid))
            .unwrap_or(TrustLevel::Unknown);
        by_device.max(by_group)
    }

    pub fn trusts(&self, id: &str, at_least: TrustLevel) -> bool {
        self.trust_level(id) >= at_least
    }

    pub fn set_trust(&self, id: &str, level: TrustLevel) {
        let Some(dgid) = dgid_of(id) else {
            return;
        };
        let mut trust = self.trust.lock().unwrap();
        trust.set(dgid, level);
        if let Some(group) = self.peer_dgids.lock().unwrap().get(id) {
            trust.set(*group, level);
        }
        if self.store_dir.is_dir() {
            let _ = trust.save(&self.store_dir);
        }
    }

    fn learn_group(&self, id: &str, group: Dgid) {
        let Some(device) = dgid_of(id) else {
            return;
        };
        self.peer_dgids
            .lock()
            .unwrap()
            .insert(id.to_string(), group);
        let mut trust = self.trust.lock().unwrap();
        if reconcile_trust(&mut trust, device, group) && self.store_dir.is_dir() {
            let _ = trust.save(&self.store_dir);
        }
    }

    pub fn trusted_peers(&self) -> Vec<(String, TrustLevel)> {
        let known = self.known.lock().unwrap();
        let trust = self.trust.lock().unwrap();
        known
            .keys()
            .map(|id| {
                let level = dgid_of(id)
                    .map(|dgid| trust.level(dgid))
                    .unwrap_or(TrustLevel::Unknown);
                (id.clone(), level)
            })
            .collect()
    }

    pub fn want(&self, name: &str) {
        self.wanted.lock().unwrap().insert(name.to_string());
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    pub fn seed(&self, value: &str) -> Result<String, Box<dyn Error>> {
        let value = value.trim();
        if let Some(base) = gateway_url(value) {
            let resolved = self.resolve_gateway(&base)?;
            return self.seed(&resolved);
        }
        let addr = parse_seed(value)?;
        let id = addr.id.to_string();
        if id != self.self_id {
            self.seeded.lock().unwrap().insert(id.clone());
            if self.trust_level(&id) < TrustLevel::Cache {
                self.set_trust(&id, TrustLevel::Cache);
            }
        }
        if id == self.self_id {
            return Ok(id);
        }
        self.forgotten.lock().unwrap().remove(&id);
        self.known.lock().unwrap().insert(id.clone(), addr);
        if value.parse::<BlobTicket>().is_ok() {
            peers::registry().introduce_provider(&id, value);
        } else {
            peers::registry().introduce_seed(&id);
        }
        self.version.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    pub fn addr_of(&self, id: &str) -> Option<EndpointAddr> {
        self.known.lock().unwrap().get(id).cloned()
    }

    pub fn set_table(&self, advert: Option<TableAdvert>) {
        *self.table.lock().unwrap() = advert;
        self.note_learned();
    }

    pub fn open_tables(&self) -> Vec<OpenTable> {
        self.tables
            .lock()
            .unwrap()
            .live(Instant::now(), wall_clock())
    }

    pub fn forget_table(&self, host: &str) {
        if self
            .tables
            .lock()
            .unwrap()
            .forget(host, Instant::now(), wall_clock())
        {
            self.note_learned();
        }
    }

    pub fn forget_peers(&self, ids: &[String]) {
        let seeded = self.seeded.lock().unwrap();
        let mut forgotten = self.forgotten.lock().unwrap();
        let mut known = self.known.lock().unwrap();
        let mut marks = self.marks.lock().unwrap();
        let mut peer_refs = self.peer_refs.lock().unwrap();
        let mut tables = self.tables.lock().unwrap();
        let now = Instant::now();
        let epoch = wall_clock();
        for id in ids {
            if seeded.contains(id) {
                continue;
            }
            known.remove(id);
            marks.remove(id);
            peer_refs.remove(id);
            tables.forget(id, now, epoch);
            forgotten.insert(id.clone());
        }
        drop(forgotten);
        drop(known);
        drop(marks);
        drop(peer_refs);
        drop(tables);
        self.note_learned();
    }

    pub fn known_peers(&self) -> Vec<EndpointAddr> {
        self.known.lock().unwrap().values().cloned().collect()
    }

    fn note_learned(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    fn record_addr(&self, addr: &EndpointAddr, introduced_by: Option<&str>) -> bool {
        let id = addr.id.to_string();
        if id == self.self_id {
            return false;
        }
        {
            let mut forgotten = self.forgotten.lock().unwrap();
            if !admits_introduction(&forgotten, &id, introduced_by) {
                return false;
            }
            forgotten.remove(&id);
        }
        let mut known = self.known.lock().unwrap();
        match known.get(&id) {
            Some(existing) if existing.addrs == addr.addrs || addr.addrs.is_empty() => false,
            Some(_) => {
                known.insert(id, addr.clone());
                true
            }
            None => {
                known.insert(id.clone(), addr.clone());
                drop(known);
                if let Some(from) = introduced_by {
                    peers::registry().learned_by_gossip(&id, from);
                }
                true
            }
        }
    }

    fn targets(&self) -> Vec<EndpointAddr> {
        let version = self.version.load(Ordering::Relaxed);
        let known = self.known.lock().unwrap();
        let marks = self.marks.lock().unwrap();
        let mut due: Vec<EndpointAddr> = known
            .iter()
            .filter(|(id, _)| match marks.get(*id) {
                None => true,
                Some(mark) if mark.failures >= DIAL_FAILURE_LIMIT => false,
                Some(mark) => mark.version < version || mark.at.elapsed() >= IDLE_RECHECK,
            })
            .map(|(_, addr)| addr.clone())
            .collect();
        due.truncate(FANOUT);
        due
    }

    pub fn mark(&self, id: &str, failed: bool) {
        let version = self.version.load(Ordering::Relaxed);
        let mut marks = self.marks.lock().unwrap();
        let entry = marks.entry(id.to_string()).or_insert(GossipMark {
            version: 0,
            at: Instant::now(),
            failures: 0,
        });
        entry.at = Instant::now();
        entry.version = version;
        if failed {
            entry.failures += 1;
        } else {
            entry.failures = 0;
        }
    }

    pub fn publish_status(&self) {
        let locals = local_refs(&self.store_dir);
        let peer_refs = self.peer_refs.lock().unwrap();
        let mut names: BTreeSet<String> = locals.iter().map(|item| item.name.clone()).collect();
        for adverts in peer_refs.values() {
            names.extend(adverts.iter().map(|item| item.name.clone()));
        }
        names.extend(self.wanted.lock().unwrap().iter().cloned());

        for name in names {
            let local = locals.iter().find(|item| item.name == name);
            let providers: Vec<String> = peer_refs
                .iter()
                .filter(|(_, adverts)| {
                    adverts
                        .iter()
                        .any(|advert| advert.name == name && advert.complete())
                })
                .map(|(id, _)| id.clone())
                .collect();
            peers::registry().set_ref_status(RefStatus {
                name: name.clone(),
                manifest: local.map(|item| item.manifest.clone()).unwrap_or_default(),
                held: local.map(|item| item.held).unwrap_or(0),
                total: local.map(|item| item.total).unwrap_or(0),
                providers,
            });
        }
    }

    pub fn best_provider(&self, name: &str) -> Option<(String, EndpointAddr, RefAdvert)> {
        let locals = local_refs(&self.store_dir);
        let ours = locals.iter().find(|item| item.name == name);
        let our_held = ours.map(|item| item.held).unwrap_or(0);
        let our_head = ours.map(|item| item.manifest.clone());
        let our_owner = ours.and_then(|item| item.owner.clone());
        let peer_refs = self.peer_refs.lock().unwrap();
        let known = self.known.lock().unwrap();
        let merged = self.merged_heads.lock().unwrap();
        let mut best: Option<RankedProvider> = None;
        for (id, adverts) in peer_refs.iter() {
            let Some(advert) = adverts
                .iter()
                .find(|advert| advert.name == name && owner_matches(&our_owner, &advert.owner))
            else {
                continue;
            };
            if advert.total == 0 {
                continue;
            }
            let more_held = advert.held > our_held;
            let sibling_head = advert.complete()
                && our_owner.is_some()
                && advert.owner == our_owner
                && our_head.as_deref() != Some(advert.manifest.as_str())
                && !merged.contains(&advert.manifest);
            if !more_held && !sibling_head {
                continue;
            }
            let level = self.trust_level(id);
            if level < TrustLevel::Cache {
                continue;
            }
            let Some(addr) = known.get(id) else {
                continue;
            };
            let owners_device =
                advert.owner.is_some() && dgid_of(id).map(|dgid| dgid.to_string()) == advert.owner;
            let rank = (owners_device, level, advert.held);
            if best
                .as_ref()
                .is_none_or(|(current, _, _, _)| rank > *current)
            {
                best = Some((rank, id.clone(), addr.clone(), advert.clone()));
            }
        }
        best.map(|(_, id, addr, advert)| (id, addr, advert))
    }

    pub fn note_merged(&self, head: &str) {
        self.merged_heads.lock().unwrap().insert(head.to_string());
    }

    pub fn missing_everywhere(&self, name: &str) -> bool {
        let locals = local_refs(&self.store_dir);
        if locals
            .iter()
            .any(|item| item.name == name && item.total > 0)
        {
            return false;
        }
        let peer_refs = self.peer_refs.lock().unwrap();
        !peer_refs.values().any(|adverts| {
            adverts
                .iter()
                .any(|advert| advert.name == name && advert.total > 0)
        })
    }

    pub fn wins_backfill_election(&self) -> bool {
        let known = self.known.lock().unwrap();
        let marks = self.marks.lock().unwrap();
        known
            .keys()
            .filter(|id| match marks.get(*id) {
                Some(mark) => mark.failures < DIAL_FAILURE_LIMIT,
                None => false,
            })
            .all(|id| self.self_id.as_str() < id.as_str())
    }

    pub fn should_backfill(&self, name: &str) -> bool {
        if self.backfill.lock().unwrap().is_none() {
            return false;
        }
        if self.started.elapsed() < BACKFILL_SETTLE {
            return false;
        }
        if let Some(at) = self.backfilled.lock().unwrap().get(name) {
            if at.elapsed() < BACKFILL_RETRY {
                return false;
            }
        }
        self.missing_everywhere(name) && self.wins_backfill_election()
    }

    pub fn run_backfill(&self, name: &str) -> Option<Result<(), String>> {
        let backfill = self.backfill.lock().unwrap().clone()?;
        self.backfilled
            .lock()
            .unwrap()
            .insert(name.to_string(), Instant::now());
        Some(backfill(name, &self.store_dir))
    }

    pub fn note_served(&self, names: impl IntoIterator<Item = String>) {
        self.served_blobs.lock().unwrap().extend(names);
    }

    pub fn unserved_blobs(&self) -> Vec<BlobHash> {
        let served = self.served_blobs.lock().unwrap();
        let Ok(entries) = std::fs::read_dir(&self.store_dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let hash = BlobHash::parse(&name)?;
                if served.contains(&name) {
                    None
                } else {
                    Some(hash)
                }
            })
            .collect()
    }

    pub fn request_blob(&self, hash: BlobHash, provider: Option<&str>) {
        if !self.accepts_downloads() {
            return;
        }
        self.blob_wants
            .lock()
            .unwrap()
            .insert(hash.to_string(), provider.map(String::from));
        self.note_learned();
    }

    pub fn wanted_blobs(&self) -> Vec<(BlobHash, Option<String>)> {
        self.blob_wants
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(hash, hint)| Some((BlobHash::parse(hash)?, hint.clone())))
            .collect()
    }

    pub fn blob_arrived(&self, hash: BlobHash) {
        self.blob_wants.lock().unwrap().remove(&hash.to_string());
    }

    pub fn blob_providers(&self, hint: Option<&str>) -> Vec<(String, EndpointAddr)> {
        let known = self.known.lock().unwrap();
        let peer_refs = self.peer_refs.lock().unwrap();
        let mut ordered: Vec<String> = hint.map(String::from).into_iter().collect();
        for (id, adverts) in peer_refs.iter() {
            if adverts
                .iter()
                .any(|advert| advert.name.contains('/') && advert.complete())
            {
                ordered.push(id.clone());
            }
        }
        ordered.extend(known.keys().cloned());
        let mut seen = BTreeSet::new();
        ordered
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .filter_map(|id| known.get(&id).map(|addr| (id, addr.clone())))
            .take(FANOUT)
            .collect()
    }

    pub fn peer_manifests(&self, name: &str) -> Vec<(String, BlobHash)> {
        let peer_refs = self.peer_refs.lock().unwrap();
        peer_refs
            .iter()
            .flat_map(|(id, adverts)| {
                adverts
                    .iter()
                    .filter(|advert| advert.name == name)
                    .filter_map(|advert| BlobHash::parse(&advert.manifest))
                    .map(move |manifest| (id.clone(), manifest))
            })
            .collect()
    }

    pub fn missing_asset_indexes(&self, store: &BlobStore) -> Vec<(String, BlobHash)> {
        self.peer_manifests(crate::assets::REF_NAME)
            .into_iter()
            .filter(|(_, manifest)| !store.has(*manifest))
            .collect()
    }

    pub fn find_asset(&self, store: &BlobStore, key: &str) -> crate::assets::Found {
        let indexes = self
            .peer_manifests(crate::assets::REF_NAME)
            .into_iter()
            .filter_map(|(peer, manifest)| Some((peer, crate::assets::read(store, manifest)?)));
        crate::assets::find(store, indexes, key)
    }

    pub fn wanted_names(&self) -> Vec<String> {
        let mut names: BTreeSet<String> = self.wanted.lock().unwrap().iter().cloned().collect();
        for advert in local_refs(&self.store_dir) {
            names.insert(advert.name);
        }
        let peer_refs = self.peer_refs.lock().unwrap();
        names.extend(followed_names(&peer_refs, |id| {
            self.trusts(id, TrustLevel::Cache)
        }));
        names.into_iter().collect()
    }
}

impl ViewSource for Mesh {
    fn local_view(&self) -> View {
        View {
            addr: self.endpoint.addr(),
            peers: self.known_peers(),
            refs: local_refs(&self.store_dir),
            table: self.table.lock().unwrap().clone(),
            heard_tables: self
                .tables
                .lock()
                .unwrap()
                .relay(Instant::now(), wall_clock()),
            dgid: self.group_dgid().map(|dgid| dgid.to_string()),
            vouch: self
                .group
                .lock()
                .unwrap()
                .as_ref()
                .map(|(_, vouch)| vouch.clone()),
        }
    }

    fn merge(&self, from: &View) {
        let sender = from.addr.id.to_string();
        if let Some(group) = from.vouched_dgid() {
            self.learn_group(&sender, group);
        }
        let mut learned = self.record_addr(&from.addr, None);
        for addr in &from.peers {
            if self.record_addr(addr, Some(&sender)) {
                learned = true;
            }
        }
        {
            let now = Instant::now();
            let epoch = wall_clock();
            let known = self.known.lock().unwrap();
            let mut tables = self.tables.lock().unwrap();
            if tables.hear(&sender, from.table.as_ref(), now, epoch) {
                learned = true;
            }
            if tables.learn(
                &sender,
                &from.heard_tables,
                &self.self_id,
                |id| known.contains_key(id),
                now,
                epoch,
            ) {
                learned = true;
            }
        }

        peers::registry().note_advertised_refs(
            &sender,
            from.refs.iter().map(|item| item.name.clone()).collect(),
        );

        let mut peer_refs = self.peer_refs.lock().unwrap();
        let changed = match peer_refs.get(&sender) {
            Some(existing) => !same_adverts(existing, &from.refs),
            None => true,
        };
        peer_refs.insert(sender, from.refs.clone());
        drop(peer_refs);

        if learned || changed {
            self.note_learned();
        }
    }
}

type RankedProvider = ((bool, TrustLevel, u32), String, EndpointAddr, RefAdvert);

pub fn reconcile_trust(trust: &mut Trust, device: Dgid, group: Dgid) -> bool {
    let by_device = trust.level(device);
    let by_group = trust.level(group);
    if by_device == by_group || by_device == TrustLevel::Unknown && by_group == TrustLevel::Unknown
    {
        return false;
    }
    let level = by_device.max(by_group);
    if by_device < level {
        trust.set(device, level);
    }
    if by_group < level {
        trust.set(group, level);
    }
    true
}

pub fn owner_matches(ours: &Option<String>, theirs: &Option<String>) -> bool {
    match (ours, theirs) {
        (Some(mine), Some(their)) => mine == their,
        _ => true,
    }
}

pub fn followed_names(
    peer_refs: &BTreeMap<String, Vec<RefAdvert>>,
    trusted: impl Fn(&str) -> bool,
) -> BTreeSet<String> {
    peer_refs
        .iter()
        .filter(|(id, _)| trusted(id))
        .flat_map(|(_, adverts)| adverts.iter().map(|advert| advert.name.clone()))
        .collect()
}

fn same_adverts(left: &[RefAdvert], right: &[RefAdvert]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right).all(|(a, b)| {
        a.name == b.name
            && a.manifest == b.manifest
            && a.total == b.total
            && a.held == b.held
            && a.owner == b.owner
    })
}

#[cfg(feature = "native")]
pub async fn pull_ref(
    endpoint: &Endpoint,
    iroh_store: &FsStore,
    store: &BlobStore,
    provider: &EndpointAddr,
    name: &str,
    manifest_hash: BlobHash,
    mut progress: impl FnMut(usize, usize),
) -> Result<u64, Box<dyn Error>> {
    if !safe_ref_name(name) {
        return Err(format!("refusing to pull unsafe ref name {name:?}").into());
    }
    let downloader = iroh_store.downloader(endpoint);
    let id = provider.id;
    let peer = id.to_string();
    let mut bytes_pulled = 0u64;

    if !store.has(manifest_hash) {
        bytes_pulled += pull_blob(&downloader, iroh_store, store, &peer, id, manifest_hash).await?;
    }

    let wanted = referenced(store, manifest_hash);
    let total = wanted.len();
    for (index, hash) in wanted.into_iter().enumerate() {
        if !store.has(hash) {
            bytes_pulled += pull_blob(&downloader, iroh_store, store, &peer, id, hash).await?;
        }
        progress(index + 1, total);
    }

    collection::merge(store, name, manifest_hash)?;
    peers::registry().note_ref(&peer, name);
    Ok(bytes_pulled)
}

#[cfg(feature = "native")]
async fn pull_blob(
    downloader: &iroh_blobs::api::downloader::Downloader,
    iroh_store: &iroh_blobs::api::Store,
    store: &BlobStore,
    peer: &str,
    id: EndpointId,
    hash: BlobHash,
) -> Result<u64, Box<dyn Error>> {
    downloader.download(iroh_hash(hash), Some(id)).await?;
    let size = crate::blobs::take(iroh_store, store, hash).await?;
    peers::registry().add_payload_received(peer, size);
    Ok(size)
}

#[cfg(feature = "native")]
pub async fn fetch_blob(
    mesh: &Arc<Mesh>,
    endpoint: &Endpoint,
    iroh_store: &iroh_blobs::api::Store,
    hash: BlobHash,
    hint: Option<&str>,
) -> Result<u64, String> {
    let store = BlobStore::open(&mesh.store_dir).map_err(|error| error.to_string())?;
    if store.has(hash) {
        return Ok(0);
    }
    let downloader = iroh_store.downloader(endpoint);
    let mut last = String::from("no peer to ask");
    for (id, addr) in mesh.blob_providers(hint) {
        match pull_blob(&downloader, iroh_store, &store, &id, addr.id, hash).await {
            Ok(size) => {
                peers::registry().set_outcome(
                    &id,
                    format!("served blob {} ({})", hash, peers::format_bytes(size)),
                );
                mesh.blob_arrived(hash);
                return Ok(size);
            }
            Err(error) => last = format!("{id}: {error}"),
        }
    }
    Err(last)
}

#[cfg(feature = "native")]
async fn fetch_wanted_blobs(mesh: &Arc<Mesh>, endpoint: &Endpoint, iroh_store: &FsStore) {
    if !mesh.accepts_downloads() {
        return;
    }
    let wants = mesh.wanted_blobs();
    if wants.is_empty() {
        return;
    }
    let Ok(store) = BlobStore::open(&mesh.store_dir) else {
        return;
    };
    let downloader = iroh_store.downloader(endpoint);
    for (hash, hint) in wants {
        if store.has(hash) {
            mesh.blob_arrived(hash);
            continue;
        }
        for (id, addr) in mesh.blob_providers(hint.as_deref()) {
            match pull_blob(&downloader, iroh_store, &store, &id, addr.id, hash).await {
                Ok(bytes) => {
                    peers::registry().set_outcome(
                        &id,
                        format!(
                            "served wanted blob {} ({})",
                            hash,
                            peers::format_bytes(bytes)
                        ),
                    );
                    mesh.blob_arrived(hash);
                    break;
                }
                Err(_) => continue,
            }
        }
    }
}

pub fn prune_dead_peers(mesh: &Arc<Mesh>) {
    let dropped = peers::registry().prune_dead(peers::DEAD_AFTER);
    if dropped.is_empty() {
        return;
    }
    mesh.forget_peers(&dropped);
}

pub async fn gossip_round(mesh: &Arc<Mesh>, endpoint: &Endpoint) {
    let ours = mesh.local_view();
    for target in mesh.targets() {
        let id = target.id.to_string();
        match gossip::exchange(endpoint, target, &ours).await {
            Ok(theirs) => {
                mesh.merge(&theirs);
                mesh.mark(&id, false);
                peers::registry().note_dial_success(&id);
            }
            Err(error) => {
                mesh.mark(&id, true);
                peers::registry().note_dial_failure(&id, error.to_string());
            }
        }
    }
    mesh.publish_status();
}

#[cfg(feature = "native")]
pub async fn round(mesh: &Arc<Mesh>, endpoint: &Endpoint, iroh_store: &FsStore, dir: &Path) {
    mesh.refresh_membership();
    gossip_round(mesh, endpoint).await;
    import_new_blobs(mesh, iroh_store).await;
    fetch_wanted_blobs(mesh, endpoint, iroh_store).await;
    converge(mesh, endpoint, iroh_store, dir).await;
    prune_dead_peers(mesh);
    mesh.publish_status();
    mesh.persist_peers();
    mesh.heartbeat();
}

#[cfg(feature = "native")]
async fn import_new_blobs(mesh: &Arc<Mesh>, iroh_store: &FsStore) {
    let fresh = mesh.unserved_blobs();
    if fresh.is_empty() {
        return;
    }
    let Ok(store) = BlobStore::open(&mesh.store_dir) else {
        return;
    };
    let mut served = Vec::new();
    for hash in fresh {
        if crate::blobs::reference(iroh_store, &store, hash)
            .await
            .is_ok()
        {
            served.push(hash.to_string());
        }
    }
    mesh.note_served(served);
}

#[cfg(feature = "native")]
async fn converge(mesh: &Arc<Mesh>, endpoint: &Endpoint, iroh_store: &FsStore, dir: &Path) {
    if !mesh.replicates() {
        return;
    }
    let Ok(store) = BlobStore::open(dir) else {
        return;
    };
    for name in mesh.wanted_names() {
        if !safe_ref_name(&name) {
            continue;
        }
        let Some((id, addr, advert)) = mesh.best_provider(&name) else {
            if mesh.should_backfill(&name) {
                backfill(mesh, &name).await;
            }
            continue;
        };
        let Some(manifest) = BlobHash::parse(&advert.manifest) else {
            continue;
        };
        match pull_ref(
            endpoint,
            iroh_store,
            &store,
            &addr,
            &name,
            manifest,
            |_, _| {},
        )
        .await
        {
            Ok(bytes) if bytes > 0 => {
                mesh.note_merged(&advert.manifest);
                peers::registry().set_outcome(
                    &id,
                    format!("pulled {} for ref {name}", peers::format_bytes(bytes)),
                );
                mesh.note_learned();
            }
            Ok(_) => {
                mesh.note_merged(&advert.manifest);
                mesh.note_learned();
            }
            Err(error) => {
                peers::registry().set_outcome(&id, format!("pull of {name} failed: {error}"));
            }
        }
    }
}

#[cfg(feature = "native")]
async fn backfill(mesh: &Arc<Mesh>, name: &str) {
    let mesh = mesh.clone();
    let name = name.to_string();
    let outcome = tokio::task::spawn_blocking(move || mesh.run_backfill(&name)).await;
    if let Ok(Some(Err(error))) = outcome {
        eprintln!("scryfall backfill failed: {error}");
    }
}

#[cfg(feature = "native")]
pub fn spawn_loop(mesh: Arc<Mesh>, endpoint: Endpoint, iroh_store: FsStore, dir: PathBuf) {
    n0_future::task::spawn(async move {
        loop {
            n0_future::time::sleep(ROUND_INTERVAL).await;
            if endpoint.is_closed() {
                break;
            }
            round(&mesh, &endpoint, &iroh_store, &dir).await;
        }
    });
}

pub fn spawn_gossip_loop(mesh: Arc<Mesh>, endpoint: Endpoint) {
    n0_future::task::spawn(async move {
        loop {
            n0_future::time::sleep(ROUND_INTERVAL).await;
            if endpoint.is_closed() {
                break;
            }
            gossip_round(&mesh, &endpoint).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize)]
    struct LegacyCard {
        name: String,
        image: String,
    }

    #[derive(serde::Serialize)]
    struct LegacyManifest {
        set: String,
        cards: Vec<LegacyCard>,
    }

    fn advert(name: &str, total: u32, held: u32) -> RefAdvert {
        RefAdvert {
            name: name.into(),
            manifest: "ab".repeat(32),
            total,
            held,
            owner: None,
        }
    }

    #[test]
    fn adverts_compare_by_content_so_gossip_can_go_quiet() {
        let left = vec![advert("hob", 194, 194)];
        assert!(same_adverts(&left, &[advert("hob", 194, 194)]));
        assert!(!same_adverts(&left, &[advert("hob", 194, 100)]));
        assert!(!same_adverts(&left, &[]));
        let mut owned = advert("hob", 194, 194);
        owned.owner = Some("dgid:aa".into());
        assert!(!same_adverts(&left, std::slice::from_ref(&owned)));
    }

    #[test]
    fn a_ref_follows_its_owner_once_it_has_one() {
        let mine = Some("dgid:me".to_string());
        assert!(owner_matches(&mine, &Some("dgid:me".into())));
        assert!(!owner_matches(&mine, &Some("dgid:them".into())));
        assert!(owner_matches(&mine, &None));
        assert!(owner_matches(&None, &Some("dgid:them".into())));
        assert!(owner_matches(&None, &None));
    }

    #[test]
    fn a_forgotten_peer_returns_only_by_speaking_to_us_not_by_hearsay() {
        let forgotten: BTreeSet<String> = ["dead".to_string()].into_iter().collect();
        assert!(!admits_introduction(&forgotten, "dead", Some("friend")));
        assert!(admits_introduction(&forgotten, "dead", None));
        assert!(admits_introduction(&forgotten, "stranger", Some("friend")));
    }

    #[test]
    fn completeness_of_an_absent_manifest_is_zero_not_a_panic() {
        let dir = std::env::temp_dir().join(format!("spirit-mesh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = BlobStore::open(&dir).unwrap();
        let absent = BlobHash::of(b"nothing here");
        assert_eq!(completeness(&store, absent), (0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn election_winner(self_id: &str, reachable: &[&str], unreachable: &[&str]) -> bool {
        let mut known: BTreeMap<String, ()> = BTreeMap::new();
        let mut marks: BTreeMap<String, u32> = BTreeMap::new();
        for id in reachable {
            known.insert((*id).to_string(), ());
            marks.insert((*id).to_string(), 0);
        }
        for id in unreachable {
            known.insert((*id).to_string(), ());
            marks.insert((*id).to_string(), DIAL_FAILURE_LIMIT);
        }
        known
            .keys()
            .filter(|id| marks.get(*id).copied().unwrap_or(DIAL_FAILURE_LIMIT) < DIAL_FAILURE_LIMIT)
            .all(|id| self_id < id.as_str())
    }

    #[test]
    fn exactly_one_reachable_node_wins_the_backfill_election() {
        let ids = ["aa", "bb", "cc"];
        let winners = ids
            .iter()
            .filter(|self_id| {
                let others: Vec<&str> = ids.iter().filter(|id| id != self_id).copied().collect();
                election_winner(self_id, &others, &[])
            })
            .count();
        assert_eq!(winners, 1);
        assert!(election_winner("aa", &["bb", "cc"], &[]));
        assert!(!election_winner("bb", &["aa", "cc"], &[]));
    }

    #[test]
    fn an_unreachable_lower_node_does_not_block_the_election() {
        assert!(election_winner("bb", &["cc"], &["aa"]));
    }

    #[test]
    fn a_lone_node_wins_its_own_election() {
        assert!(election_winner("zz", &[], &[]));
    }

    #[test]
    fn a_gateway_url_is_recognised_as_a_seed_and_pointed_at_its_status() {
        assert_eq!(
            gateway_url("https://dev1.example.net/"),
            Some("https://dev1.example.net".to_string())
        );
        assert_eq!(
            gateway_url(" http://127.0.0.1:8090 "),
            Some("http://127.0.0.1:8090".to_string())
        );
        assert_eq!(gateway_url("endpointabc"), None);
        assert_eq!(gateway_url("ab".repeat(32).as_str()), None);
        assert_eq!(
            status_url("http://127.0.0.1:8090/"),
            "http://127.0.0.1:8090/gateway/status"
        );
        assert!(is_seed("https://dev1.example.net").is_ok());
        assert!(is_seed("not a seed").is_err());
    }

    #[test]
    fn a_gateway_status_resolves_to_its_ticket_or_its_node_id() {
        let secret = iroh::SecretKey::from_bytes(&[5; 32]);
        let addr = EndpointAddr::new(secret.public());
        let id = addr.id.to_string();
        let ticket = EndpointTicket::from(addr).to_string();
        let full = serde_json::json!({
            "node_id": id,
            "ticket": ticket,
            "dgid": "dgid:whatever",
            "peers": 3,
            "refs": [],
        })
        .to_string();
        assert_eq!(seed_from_status(&full).unwrap(), ticket);
        assert_eq!(parse_seed(&ticket).unwrap().id.to_string(), id);
        let old = serde_json::json!({ "node_id": id, "peers": 0, "refs": [] }).to_string();
        assert_eq!(seed_from_status(&old).unwrap(), id);
        let broken = serde_json::json!({ "node_id": "nope", "ticket": "garbage" }).to_string();
        assert!(seed_from_status(&broken).is_err());
        assert!(seed_from_status("{}").is_err());
        assert!(seed_from_status("not json").is_err());
    }

    #[test]
    fn every_ticket_shape_parses_to_the_same_peer() {
        let secret = iroh::SecretKey::from_bytes(&[7; 32]);
        let addr = EndpointAddr::new(secret.public());
        let id = addr.id.to_string();
        let endpoint_ticket = EndpointTicket::from(addr.clone()).to_string();
        assert!(endpoint_ticket.starts_with("endpoint"));
        assert_eq!(parse_seed(&endpoint_ticket).unwrap().id.to_string(), id);
        assert_eq!(parse_seed(&id).unwrap().id.to_string(), id);
        let blob_ticket = BlobTicket::new(
            addr,
            iroh_blobs::Hash::from_bytes([9; 32]),
            iroh_blobs::BlobFormat::Raw,
        )
        .to_string();
        assert_eq!(parse_seed(&blob_ticket).unwrap().id.to_string(), id);
        assert!(parse_seed("not a ticket").is_err());
    }

    #[test]
    fn module_refs_advertise_under_the_modules_prefix() {
        let dir = std::env::temp_dir().join(format!("spirit-mesh-modules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = BlobStore::open(&dir).unwrap();
        let module = b"hardened module bytes";
        let manifest = spirit_core::modules::ModuleManifest {
            name: "riftbound".into(),
            kind: spirit_core::modules::ModuleKind::Plugin,
            abi_version: 0,
            display: "Riftbound".into(),
            version: "0.1.0".into(),
            module: BlobHash::of(module).to_string(),
        };
        spirit_core::modules::publish_module(&store, &manifest, module).unwrap();
        let refs = local_refs(&dir);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "modules/riftbound");
        assert_eq!((refs[0].total, refs[0].held), (1, 1));
        assert!(refs[0].complete());
        std::fs::remove_file(dir.join(BlobHash::of(module).to_string())).unwrap();
        let refs = local_refs(&dir);
        assert_eq!((refs[0].total, refs[0].held), (1, 0));
        assert!(!refs[0].complete());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_module_version_collection_advertises_every_blob_it_needs() {
        let dir =
            std::env::temp_dir().join(format!("spirit-mesh-collection-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = BlobStore::open(&dir).unwrap();
        let identity = spirit_core::identity::load_or_create(&dir).unwrap();
        let wasm = b"hardened module bytes";
        spirit_schema::modules::publish(
            &store,
            &identity,
            &spirit_schema::modules::Module::new(
                "riftbound",
                spirit_schema::modules::Role::Plugin,
                "0.1.0",
                3,
            ),
            &spirit_core::record::Tdr::new("wasm-harden", &("test",)).unwrap(),
            wasm,
        )
        .unwrap();

        let refs = local_refs(&dir);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "modules/riftbound");
        assert!(refs[0].complete());
        assert!(refs[0].total >= 4);
        assert_eq!(refs[0].total, refs[0].held);

        std::fs::remove_file(dir.join(BlobHash::of(wasm).to_string())).unwrap();
        let refs = local_refs(&dir);
        assert!(!refs[0].complete());
        assert_eq!(refs[0].held, refs[0].total - 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn card_and_module_refs_advertise_side_by_side() {
        let dir = std::env::temp_dir().join(format!("spirit-mesh-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = BlobStore::open(&dir).unwrap();
        let art = store.put(b"jpeg").unwrap();
        let cards = LegacyManifest {
            set: "hob".into(),
            cards: vec![LegacyCard {
                name: "Attercop".into(),
                image: art.to_string(),
            }],
        };
        let mut encoded = Vec::new();
        ciborium::into_writer(&cards, &mut encoded).unwrap();
        let cards_hash = store.put(&encoded).unwrap();
        std::fs::create_dir_all(store.root().join("refs")).unwrap();
        std::fs::write(
            store.root().join("refs").join("hob"),
            format!("{cards_hash}\n"),
        )
        .unwrap();
        let module = b"engine bytes";
        let manifest = spirit_core::modules::ModuleManifest {
            name: "engine".into(),
            kind: spirit_core::modules::ModuleKind::Engine,
            abi_version: 0,
            display: "agni engine".into(),
            version: "0.1.0".into(),
            module: BlobHash::of(module).to_string(),
        };
        spirit_core::modules::publish_module(&store, &manifest, module).unwrap();
        let mut names: Vec<String> = local_refs(&dir).into_iter().map(|r| r.name).collect();
        names.sort();
        assert_eq!(names, vec!["hob".to_string(), "modules/engine".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ref_name_safety_permits_cards_and_modules_but_not_traversal() {
        assert!(safe_ref_name("hob"));
        assert!(safe_ref_name("modules/engine"));
        assert!(safe_ref_name("modules/riftbound-0.1"));
        assert!(!safe_ref_name("modules/../../identity"));
        assert!(!safe_ref_name("../escape"));
        assert!(!safe_ref_name("modules/"));
        assert!(safe_ref_name("cards/mtg"));
        assert!(!safe_ref_name("a/b/c/d"));
        assert!(!safe_ref_name(".hidden"));
        assert!(!safe_ref_name(""));
    }

    #[test]
    fn only_a_trusted_peer_can_introduce_a_ref_name() {
        let mut peer_refs = BTreeMap::new();
        peer_refs.insert(
            "trusted-peer".to_string(),
            vec![advert("hob", 1, 1), advert("modules/engine", 1, 1)],
        );
        peer_refs.insert(
            "stranger".to_string(),
            vec![advert("modules/riftbound", 1, 1)],
        );

        let followed = followed_names(&peer_refs, |id| id == "trusted-peer");
        assert!(followed.contains("hob"));
        assert!(followed.contains("modules/engine"));
        assert!(!followed.contains("modules/riftbound"));

        assert_eq!(followed_names(&peer_refs, |_| false).len(), 0);
        assert_eq!(followed_names(&peer_refs, |_| true).len(), 3);
    }

    #[test]
    fn an_unparsable_peer_id_is_never_trusted() {
        assert!(dgid_of("not-an-endpoint-id").is_none());
    }

    #[test]
    fn local_refs_of_an_empty_store_is_empty() {
        let dir = std::env::temp_dir().join(format!("spirit-mesh-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        BlobStore::open(&dir).unwrap();
        assert!(local_refs(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vouched_group_and_its_device_share_the_higher_trust_level() {
        let device = Dgid::from_bytes([1; 32]);
        let group = Dgid::from_bytes([2; 32]);
        let mut trust = Trust::new();
        assert!(!reconcile_trust(&mut trust, device, group));
        trust.set(device, TrustLevel::Cache);
        assert!(reconcile_trust(&mut trust, device, group));
        assert_eq!(trust.level(group), TrustLevel::Cache);
        trust.set(group, TrustLevel::Mesh);
        assert!(reconcile_trust(&mut trust, device, group));
        assert_eq!(trust.level(device), TrustLevel::Mesh);
        assert!(!reconcile_trust(&mut trust, device, group));
    }

    #[test]
    fn remembered_peers_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("spirit-peers-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let addr = EndpointAddr::new(iroh::SecretKey::from_bytes(&[9; 32]).public());
        let remembered = RememberedPeers {
            peers: vec![RememberedPeer {
                id: addr.id.to_string(),
                addr: addr.clone(),
                first_seen: 1_700_000_000,
                last_activity: 1_700_000_100,
                refs: vec!["hob".into()],
                ticket: None,
            }],
            forgotten: vec!["dead".into()],
        };
        std::fs::write(
            dir.join(PEERS_FILE),
            serde_json::to_string(&remembered).unwrap(),
        )
        .unwrap();
        let back = read_remembered(&dir);
        assert_eq!(back.peers.len(), 1);
        assert_eq!(back.peers[0].addr.id, addr.id);
        assert_eq!(back.peers[0].refs, vec!["hob"]);
        assert_eq!(back.forgotten, vec!["dead"]);
        assert!(read_remembered(Path::new("/nonexistent")).peers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
