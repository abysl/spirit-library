use n0_future::time::SystemTime;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkState {
    NeverReached,
    Connected,
    Reachable,
    Disconnected,
}

impl LinkState {
    pub fn label(self) -> &'static str {
        match self {
            LinkState::NeverReached => "never reached",
            LinkState::Connected => "connected",
            LinkState::Reachable => "reachable",
            LinkState::Disconnected => "disconnected",
        }
    }
}

pub const REACHABLE_WINDOW: Duration = Duration::from_secs(60);
pub const DEAD_AFTER: Duration = Duration::from_secs(60 * 60);

pub fn link_state(live: bool, ever_connected: bool, since_activity: Option<Duration>) -> LinkState {
    if live {
        return LinkState::Connected;
    }
    if !ever_connected {
        return LinkState::NeverReached;
    }
    match since_activity {
        Some(age) if age <= REACHABLE_WINDOW => LinkState::Reachable,
        _ => LinkState::Disconnected,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathKind {
    Unknown,
    Direct,
    Relay,
    Mixed,
}

impl PathKind {
    pub fn label(self) -> &'static str {
        match self {
            PathKind::Unknown => "unknown",
            PathKind::Direct => "direct",
            PathKind::Relay => "relay",
            PathKind::Mixed => "direct+relay",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Discovery {
    Ticket,
    Inbound,
    Gossip(String),
    Remembered,
}

impl Discovery {
    pub fn label(&self) -> String {
        match self {
            Discovery::Ticket => "scanned ticket".into(),
            Discovery::Inbound => "dialed us".into(),
            Discovery::Remembered => "remembered from the last run".into(),
            Discovery::Gossip(from) => {
                format!(
                    "introduced by {}",
                    from.chars().take(12).collect::<String>()
                )
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RefStatus {
    pub name: String,
    pub manifest: String,
    pub held: u32,
    pub total: u32,
    pub providers: Vec<String>,
}

impl RefStatus {
    pub fn complete(&self) -> bool {
        self.total > 0 && self.held >= self.total
    }

    pub fn label(&self) -> String {
        if self.total == 0 {
            format!("{} — missing", self.name)
        } else if self.complete() {
            format!("{} — complete ({} blobs)", self.name, self.total)
        } else {
            format!("{} — partial ({}/{})", self.name, self.held, self.total)
        }
    }
}

#[derive(Clone, Debug)]
pub struct PeerSnapshot {
    pub id: String,
    pub we_served_them: bool,
    pub we_fetched_from_them: bool,
    pub wire_bytes_sent: u64,
    pub wire_bytes_received: u64,
    pub payload_bytes_received: u64,
    pub wire_bytes_known: bool,
    pub state: LinkState,
    pub live_connections: usize,
    pub total_connections: u64,
    pub rtt: Option<Duration>,
    pub active_addrs: Vec<String>,
    pub inactive_addrs: Vec<String>,
    pub addrs_checked: Option<SystemTime>,
    pub first_seen: SystemTime,
    pub last_activity: SystemTime,
    pub introduced_by_ref: Option<String>,
    pub ticket: Option<String>,
    pub outcome: Option<String>,
    pub discovery: Discovery,
    pub dial_failures: u32,
    pub last_dial_error: Option<String>,
    pub refs_advertised: Vec<String>,
}

impl PeerSnapshot {
    pub fn short_id(&self) -> String {
        self.id.chars().take(12).collect()
    }

    pub fn path_kind(&self) -> PathKind {
        let relay = self
            .active_addrs
            .iter()
            .any(|addr| addr.starts_with("relay"));
        let direct = self
            .active_addrs
            .iter()
            .any(|addr| addr.starts_with("direct"));
        match (direct, relay) {
            (true, true) => PathKind::Mixed,
            (true, false) => PathKind::Direct,
            (false, true) => PathKind::Relay,
            (false, false) => PathKind::Unknown,
        }
    }

    pub fn has_active_path(&self) -> bool {
        !self.active_addrs.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct ConnectionKey {
    id: String,
    serial: u64,
}

#[derive(Debug)]
struct PeerRecord {
    we_served_them: bool,
    we_fetched_from_them: bool,
    wire_bytes_sent: u64,
    wire_bytes_received: u64,
    payload_bytes_received: u64,
    wire_bytes_known: bool,
    live: BTreeMap<u64, (u64, u64)>,
    total_connections: u64,
    ever_connected: bool,
    rtt: Option<Duration>,
    active_addrs: Vec<String>,
    inactive_addrs: Vec<String>,
    addrs_checked: Option<SystemTime>,
    first_seen: SystemTime,
    last_activity: SystemTime,
    introduced_by_ref: Option<String>,
    ticket: Option<String>,
    outcome: Option<String>,
    discovery: Discovery,
    dial_failures: u32,
    last_dial_error: Option<String>,
    refs_advertised: Vec<String>,
}

impl Default for PeerRecord {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            we_served_them: false,
            we_fetched_from_them: false,
            wire_bytes_sent: 0,
            wire_bytes_received: 0,
            payload_bytes_received: 0,
            wire_bytes_known: false,
            live: BTreeMap::new(),
            total_connections: 0,
            ever_connected: false,
            rtt: None,
            active_addrs: Vec::new(),
            inactive_addrs: Vec::new(),
            addrs_checked: None,
            first_seen: now,
            last_activity: now,
            introduced_by_ref: None,
            ticket: None,
            outcome: None,
            discovery: Discovery::Inbound,
            dial_failures: 0,
            last_dial_error: None,
            refs_advertised: Vec::new(),
        }
    }
}

impl PeerRecord {
    fn snapshot(&self, id: &str) -> PeerSnapshot {
        let state = link_state(
            !self.live.is_empty(),
            self.ever_connected,
            self.last_activity.elapsed().ok(),
        );
        PeerSnapshot {
            id: id.to_string(),
            we_served_them: self.we_served_them,
            we_fetched_from_them: self.we_fetched_from_them,
            wire_bytes_sent: self.wire_bytes_sent,
            wire_bytes_received: self.wire_bytes_received,
            payload_bytes_received: self.payload_bytes_received,
            wire_bytes_known: self.wire_bytes_known,
            state,
            live_connections: self.live.len(),
            total_connections: self.total_connections,
            rtt: self.rtt,
            active_addrs: self.active_addrs.clone(),
            inactive_addrs: self.inactive_addrs.clone(),
            addrs_checked: self.addrs_checked,
            first_seen: self.first_seen,
            last_activity: self.last_activity,
            introduced_by_ref: self.introduced_by_ref.clone(),
            ticket: self.ticket.clone(),
            outcome: self.outcome.clone(),
            discovery: self.discovery.clone(),
            dial_failures: self.dial_failures,
            last_dial_error: self.last_dial_error.clone(),
            refs_advertised: self.refs_advertised.clone(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PeerRegistry {
    peers: Arc<Mutex<BTreeMap<String, PeerRecord>>>,
    refs: Arc<Mutex<BTreeMap<String, RefStatus>>>,
    serial: Arc<AtomicU64>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn edit<R>(&self, id: &str, edit: impl FnOnce(&mut PeerRecord) -> R) -> R {
        let mut peers = self.peers.lock().unwrap();
        let record = peers.entry(id.to_string()).or_default();
        edit(record)
    }

    fn heard<R>(&self, id: &str, edit: impl FnOnce(&mut PeerRecord) -> R) -> R {
        self.edit(id, |record| {
            record.last_activity = SystemTime::now();
            edit(record)
        })
    }

    pub fn introduce_provider(&self, id: &str, ticket: &str) {
        self.edit(id, |record| {
            record.we_fetched_from_them = true;
            record.ticket = Some(ticket.to_string());
            record.discovery = Discovery::Ticket;
        });
    }

    pub fn remember(
        &self,
        id: &str,
        first_seen: SystemTime,
        last_activity: SystemTime,
        refs: Vec<String>,
        ticket: Option<String>,
    ) {
        let mut peers = self.peers.lock().unwrap();
        if peers.contains_key(id) {
            return;
        }
        let record = PeerRecord {
            discovery: Discovery::Remembered,
            first_seen,
            last_activity,
            refs_advertised: refs,
            ticket,
            ..Default::default()
        };
        peers.insert(id.to_string(), record);
    }

    pub fn introduce_seed(&self, id: &str) {
        self.edit(id, |record| {
            record.discovery = Discovery::Ticket;
        });
    }

    pub fn note_ref(&self, id: &str, name: &str) {
        self.heard(id, |record| {
            record.introduced_by_ref = Some(name.to_string());
        });
    }

    pub fn set_outcome(&self, id: &str, outcome: String) {
        self.edit(id, |record| {
            record.outcome = Some(outcome);
        });
    }

    pub fn add_payload_received(&self, id: &str, bytes: u64) {
        self.heard(id, |record| {
            record.payload_bytes_received += bytes;
            record.ever_connected = true;
            record.we_fetched_from_them = true;
        });
    }

    pub fn connection_opened(&self, id: &str) -> ConnectionKey {
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        self.heard(id, |record| {
            record.we_served_them = true;
            record.ever_connected = true;
            record.total_connections += 1;
            record.live.insert(serial, (0, 0));
        });
        ConnectionKey {
            id: id.to_string(),
            serial,
        }
    }

    pub fn observe(&self, key: &ConnectionKey, sent: u64, received: u64, rtt: Option<Duration>) {
        self.heard(&key.id, |record| {
            let previous = record.live.get(&key.serial).copied().unwrap_or((0, 0));
            record.wire_bytes_sent += sent.saturating_sub(previous.0);
            record.wire_bytes_received += received.saturating_sub(previous.1);
            record.wire_bytes_known = true;
            if let Some(slot) = record.live.get_mut(&key.serial) {
                *slot = (sent, received);
            }
            if rtt.is_some() {
                record.rtt = rtt;
            }
        });
    }

    pub fn connection_closed(&self, key: &ConnectionKey) {
        self.heard(&key.id, |record| {
            record.live.remove(&key.serial);
        });
    }

    pub fn set_addrs(&self, id: &str, active: Vec<String>, inactive: Vec<String>) {
        let mut peers = self.peers.lock().unwrap();
        let Some(record) = peers.get_mut(id) else {
            return;
        };
        record.active_addrs = active;
        record.inactive_addrs = inactive;
        record.addrs_checked = Some(SystemTime::now());
    }

    pub fn learned_by_gossip(&self, id: &str, from: &str) {
        self.edit(id, |record| {
            if record.discovery == Discovery::Inbound && record.total_connections == 0 {
                record.discovery = Discovery::Gossip(from.to_string());
            }
        });
    }

    pub fn note_dial_failure(&self, id: &str, error: String) {
        self.edit(id, |record| {
            record.dial_failures += 1;
            record.last_dial_error = Some(error);
        });
    }

    pub fn note_dial_success(&self, id: &str) {
        self.heard(id, |record| {
            record.dial_failures = 0;
            record.last_dial_error = None;
            record.ever_connected = true;
        });
    }

    pub fn note_advertised_refs(&self, id: &str, refs: Vec<String>) {
        self.heard(id, |record| {
            record.refs_advertised = refs;
        });
    }

    pub fn set_ref_status(&self, status: RefStatus) {
        self.refs
            .lock()
            .unwrap()
            .insert(status.name.clone(), status);
    }

    pub fn prune_dead(&self, older_than: Duration) -> Vec<String> {
        let mut peers = self.peers.lock().unwrap();
        let mut dropped = Vec::new();
        peers.retain(|id, record| {
            let stale = record
                .last_activity
                .elapsed()
                .map(|age| age > older_than)
                .unwrap_or(false);
            let keep = !record.live.is_empty() || !stale;
            if !keep {
                dropped.push(id.clone());
            }
            keep
        });
        dropped
    }

    pub fn ref_statuses(&self) -> Vec<RefStatus> {
        self.refs.lock().unwrap().values().cloned().collect()
    }

    pub fn ids(&self) -> Vec<String> {
        self.peers.lock().unwrap().keys().cloned().collect()
    }

    pub fn snapshot(&self) -> Vec<PeerSnapshot> {
        let peers = self.peers.lock().unwrap();
        let mut list: Vec<PeerSnapshot> = peers
            .iter()
            .map(|(id, record)| record.snapshot(id))
            .collect();
        list.sort_by_key(|peer| std::cmp::Reverse(peer.last_activity));
        list
    }
}

pub fn newly_complete(
    previous: &std::collections::BTreeSet<String>,
    statuses: &[RefStatus],
) -> (std::collections::BTreeSet<String>, Vec<String>) {
    let complete: std::collections::BTreeSet<String> = statuses
        .iter()
        .filter(|status| status.complete())
        .map(|status| status.name.clone())
        .collect();
    let fresh = complete
        .iter()
        .filter(|name| !previous.contains(*name))
        .cloned()
        .collect();
    (complete, fresh)
}

static GLOBAL: OnceLock<PeerRegistry> = OnceLock::new();

pub fn registry() -> &'static PeerRegistry {
    GLOBAL.get_or_init(PeerRegistry::new)
}

pub fn snapshot() -> Vec<PeerSnapshot> {
    registry().snapshot()
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.2} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_age(instant: SystemTime) -> String {
    let Ok(elapsed) = instant.elapsed() else {
        return "just now".into();
    };
    let seconds = elapsed.as_secs();
    if seconds < 2 {
        "just now".into()
    } else if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_we_gossiped_with_moments_ago_reads_as_reachable_not_gone() {
        assert_eq!(
            link_state(false, true, Some(Duration::from_secs(15))),
            LinkState::Reachable
        );
        assert_eq!(
            link_state(false, true, Some(Duration::from_secs(59))),
            LinkState::Reachable
        );
        assert_eq!(
            link_state(false, true, Some(Duration::from_secs(120))),
            LinkState::Disconnected
        );
    }

    #[test]
    fn a_live_connection_still_outranks_recency_and_a_stranger_is_never_reached() {
        assert_eq!(
            link_state(true, true, Some(Duration::from_secs(9999))),
            LinkState::Connected
        );
        assert_eq!(
            link_state(false, false, Some(Duration::from_secs(1))),
            LinkState::NeverReached
        );
        assert_eq!(link_state(false, true, None), LinkState::Disconnected);
    }

    #[test]
    fn pruning_drops_the_long_silent_and_keeps_the_fresh() {
        let registry = PeerRegistry::new();
        registry.note_dial_success("fresh");
        registry.note_dial_success("stale");
        {
            let mut peers = registry.peers.lock().unwrap();
            let record = peers.get_mut("stale").expect("the stale peer exists");
            record.last_activity = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        }

        let dropped = registry.prune_dead(DEAD_AFTER);
        assert_eq!(dropped, vec!["stale".to_string()]);
        let ids = registry.ids();
        assert!(ids.contains(&"fresh".to_string()));
        assert!(!ids.contains(&"stale".to_string()));
    }

    #[test]
    fn failed_dials_and_hearsay_are_not_activity_so_a_dead_peer_still_ages_out() {
        let registry = PeerRegistry::new();
        registry.note_dial_success("dead");
        {
            let mut peers = registry.peers.lock().unwrap();
            let record = peers.get_mut("dead").expect("the dead peer exists");
            record.last_activity = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        }
        registry.note_dial_failure("dead", "timed out".into());
        registry.learned_by_gossip("dead", "friend");
        registry.set_outcome("dead", "pull of hob failed".into());

        assert_eq!(registry.prune_dead(DEAD_AFTER), vec!["dead".to_string()]);
    }

    #[test]
    fn a_completed_exchange_is_activity_and_keeps_a_peer() {
        let registry = PeerRegistry::new();
        registry.note_dial_success("back");
        {
            let mut peers = registry.peers.lock().unwrap();
            let record = peers.get_mut("back").expect("the peer exists");
            record.last_activity = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        }
        registry.note_advertised_refs("back", vec!["hob".into()]);

        assert!(registry.prune_dead(DEAD_AFTER).is_empty());
    }

    #[test]
    fn pruning_never_drops_a_peer_holding_a_live_connection() {
        let registry = PeerRegistry::new();
        let key = registry.connection_opened("busy");
        {
            let mut peers = registry.peers.lock().unwrap();
            let record = peers.get_mut("busy").expect("the busy peer exists");
            record.last_activity = SystemTime::now() - Duration::from_secs(5 * 60 * 60);
        }
        assert!(registry.prune_dead(DEAD_AFTER).is_empty());
        assert!(registry.ids().contains(&"busy".to_string()));
        registry.connection_closed(&key);
    }

    #[test]
    fn pruning_an_empty_registry_is_quiet() {
        let registry = PeerRegistry::new();
        assert!(registry.prune_dead(DEAD_AFTER).is_empty());
    }

    #[test]
    fn byte_counts_accumulate_as_deltas_across_samples() {
        let registry = PeerRegistry::new();
        let key = registry.connection_opened("peer-a");
        registry.observe(&key, 1_000, 100, None);
        registry.observe(&key, 4_000, 400, None);
        registry.connection_closed(&key);

        let peer = registry.snapshot().remove(0);
        assert_eq!(peer.wire_bytes_sent, 4_000);
        assert_eq!(peer.wire_bytes_received, 400);
        assert_eq!(peer.state, LinkState::Reachable);
        assert_eq!(peer.total_connections, 1);
    }

    #[test]
    fn concurrent_connections_to_one_peer_sum_independently() {
        let registry = PeerRegistry::new();
        let first = registry.connection_opened("peer-a");
        let second = registry.connection_opened("peer-a");
        registry.observe(&first, 500, 50, None);
        registry.observe(&second, 700, 70, None);
        registry.observe(&first, 900, 90, None);

        let peer = registry.snapshot().remove(0);
        assert_eq!(peer.wire_bytes_sent, 1_600);
        assert_eq!(peer.wire_bytes_received, 160);
        assert_eq!(peer.live_connections, 2);
        assert_eq!(peer.state, LinkState::Connected);
    }

    #[test]
    fn a_peer_we_only_hold_a_ticket_for_is_never_reached() {
        let registry = PeerRegistry::new();
        registry.introduce_provider("peer-b", "blobabc");

        let peer = registry.snapshot().remove(0);
        assert_eq!(peer.state, LinkState::NeverReached);
        assert!(peer.we_fetched_from_them);
        assert!(!peer.we_served_them);
        assert!(!peer.wire_bytes_known);
        assert_eq!(peer.ticket.as_deref(), Some("blobabc"));
    }

    #[test]
    fn payload_bytes_track_the_fetch_side_where_wire_bytes_are_unavailable() {
        let registry = PeerRegistry::new();
        registry.introduce_provider("peer-c", "blobxyz");
        registry.add_payload_received("peer-c", 17_000_000);
        registry.note_ref("peer-c", "hob");

        let peer = registry.snapshot().remove(0);
        assert_eq!(peer.payload_bytes_received, 17_000_000);
        assert!(!peer.wire_bytes_known);
        assert_eq!(peer.state, LinkState::Reachable);
        assert_eq!(peer.introduced_by_ref.as_deref(), Some("hob"));
    }

    #[test]
    fn path_kind_reads_the_active_address_list() {
        let registry = PeerRegistry::new();
        registry.introduce_provider("peer-d", "blob1");
        registry.set_addrs(
            "peer-d",
            vec!["direct 192.168.1.190:41234".into()],
            vec!["relay https://euw1-1.relay.iroh.link./".into()],
        );

        let peer = registry.snapshot().remove(0);
        assert_eq!(peer.path_kind(), PathKind::Direct);
        assert!(peer.has_active_path());
    }

    #[test]
    fn a_ref_finishing_replication_is_reported_exactly_once() {
        use std::collections::BTreeSet;
        let status = |name: &str, held: u32, total: u32| RefStatus {
            name: name.into(),
            manifest: String::new(),
            held,
            total,
            providers: Vec::new(),
        };
        let previous = BTreeSet::new();
        let (seen, fresh) = newly_complete(&previous, &[status("hob", 100, 194)]);
        assert!(fresh.is_empty());
        let (seen, fresh) = newly_complete(&seen, &[status("hob", 194, 194)]);
        assert_eq!(fresh, vec!["hob".to_string()]);
        let (_, fresh) = newly_complete(&seen, &[status("hob", 194, 194)]);
        assert!(fresh.is_empty());
    }

    #[test]
    fn human_byte_formatting_crosses_each_unit() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2_048), "2.0 KiB");
        assert_eq!(format_bytes(17 * 1024 * 1024), "17.00 MiB");
    }
}
