use crate::mesh::Mesh;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use iroh_tickets::endpoint::EndpointTicket;
use serde::{Deserialize, Serialize};
use spirit_core::{identity, BlobHash, BlobStore, Dgid, Identity, TrustLevel};
use spirit_schema::device::{self, Device, Settings};
use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const ALPN: &[u8] = b"spirit-pair/0";
pub const OFFER_TTL: Duration = Duration::from_secs(600);
const MAX_MESSAGE_BYTES: usize = 1 << 16;

#[derive(Debug, Clone)]
pub struct Offer {
    pub token: [u8; 32],
    pub expires: Instant,
}

impl Offer {
    pub fn new() -> std::io::Result<Self> {
        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(std::io::Error::other)?;
        Ok(Self {
            token,
            expires: Instant::now() + OFFER_TTL,
        })
    }

    pub fn token_hex(&self) -> String {
        hex(&self.token)
    }

    pub fn live(&self) -> bool {
        Instant::now() < self.expires
    }

    pub fn seconds_left(&self) -> u64 {
        self.expires
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
}

impl Default for Offer {
    fn default() -> Self {
        Self::new().expect("the system random source works")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub ticket: String,
    pub dgid: Dgid,
    pub token: [u8; 32],
}

impl Invite {
    pub fn url(&self) -> String {
        format!(
            "spirit://pair?ticket={}&dgid={}&token={}",
            self.ticket,
            self.dgid.to_string().trim_start_matches("dgid:"),
            hex(&self.token)
        )
    }

    pub fn parse(url: &str) -> Result<Self, String> {
        let query = url
            .trim()
            .strip_prefix("spirit://pair?")
            .ok_or("a pairing link starts with spirit://pair?")?;
        let mut ticket = None;
        let mut dgid = None;
        let mut token = None;
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            match key {
                "ticket" => ticket = Some(value.to_string()),
                "dgid" => dgid = Dgid::parse(value),
                "token" => token = parse_token(value),
                _ => {}
            }
        }
        Ok(Self {
            ticket: ticket.ok_or("the pairing link has no ticket")?,
            dgid: dgid.ok_or("the pairing link has no valid dgid")?,
            token: token.ok_or("the pairing link has no valid token")?,
        })
    }
}

fn parse_token(text: &str) -> Option<[u8; 32]> {
    Dgid::parse(text).map(|dgid| *dgid.as_bytes())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRequest {
    pub token: String,
    pub device: Device,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub members: Vec<EndpointAddr>,
}

impl JoinReply {
    fn refused(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            group_secret: None,
            head: None,
            members: Vec::new(),
        }
    }
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[derive(Clone)]
pub struct PairHandler {
    mesh: Arc<Mesh>,
}

impl std::fmt::Debug for PairHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairHandler").finish()
    }
}

impl PairHandler {
    pub fn new(mesh: Arc<Mesh>) -> Self {
        Self { mesh }
    }
}

pub fn admit(mesh: &Mesh, remote: [u8; 32], request: &JoinRequest) -> JoinReply {
    let Some(token) = parse_token(&request.token) else {
        return JoinReply::refused("malformed token");
    };
    if !mesh.take_offer(&token) {
        return JoinReply::refused("no live pairing offer matches that token");
    }
    if request.device.node_id() != Some(remote) {
        return JoinReply::refused("the device record names a different node than the one dialing");
    }
    if !request.device.verify() {
        return JoinReply::refused("the device record's signature does not verify");
    }
    let dir = mesh.store_dir();
    let Some(group) = identity::load(dir) else {
        return JoinReply::refused("this node holds no group key");
    };
    let Ok(store) = BlobStore::open(dir) else {
        return JoinReply::refused("store unavailable");
    };
    let admitted = match device::admit(&store, &group, &request.device) {
        Ok(admitted) => admitted,
        Err(error) => return JoinReply::refused(error),
    };
    let remote_hex = hex(&remote);
    mesh.set_trust(&remote_hex, TrustLevel::Mesh);
    mesh.refresh_membership();
    let mut members = vec![mesh.endpoint_addr()];
    members.extend(mesh.known_peers());
    JoinReply {
        ok: true,
        error: None,
        group_secret: Some(hex(&group.secret_bytes())),
        head: Some(admitted.head.to_string()),
        members,
    }
}

impl ProtocolHandler for PairHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let remote = *connection.remote_id().as_bytes();
        let (mut send, mut recv) = connection.accept_bi().await?;
        let bytes = recv
            .read_to_end(MAX_MESSAGE_BYTES)
            .await
            .map_err(AcceptError::from_err)?;
        let reply = match ciborium::from_reader::<JoinRequest, _>(bytes.as_slice()) {
            Ok(request) => {
                let mesh = self.mesh.clone();
                tokio::task::spawn_blocking(move || admit(&mesh, remote, &request))
                    .await
                    .unwrap_or_else(|_| JoinReply::refused("admission panicked"))
            }
            Err(error) => JoinReply::refused(format!("bad join request: {error}")),
        };
        let mut out = Vec::new();
        ciborium::into_writer(&reply, &mut out).map_err(AcceptError::from_err)?;
        tokio::io::AsyncWriteExt::write_all(&mut send, &out)
            .await
            .map_err(AcceptError::from_err)?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Joined {
    pub dgid: Dgid,
    pub head: Option<BlobHash>,
    pub members: Vec<EndpointAddr>,
}

pub async fn join(
    endpoint: &Endpoint,
    store_dir: &Path,
    invite: &Invite,
) -> Result<Joined, Box<dyn Error>> {
    let device = identity::load_or_create_device(store_dir)?;
    if device.dgid().as_bytes() != endpoint.id().as_bytes() {
        return Err("the endpoint is not bound with this store's device key".into());
    }
    let record = Device::sign(&device, Settings::for_group(invite.dgid))?;
    let ticket: EndpointTicket = invite.ticket.parse()?;
    let addr: EndpointAddr = ticket.into();
    let connection = endpoint.connect(addr, ALPN).await?;
    let (mut send, mut recv) = connection.open_bi().await?;
    let mut out = Vec::new();
    ciborium::into_writer(
        &JoinRequest {
            token: hex(&invite.token),
            device: record,
        },
        &mut out,
    )?;
    tokio::io::AsyncWriteExt::write_all(&mut send, &out).await?;
    send.finish()?;
    let bytes = recv.read_to_end(MAX_MESSAGE_BYTES).await?;
    connection.close(0u32.into(), b"done");
    let reply: JoinReply = ciborium::from_reader(bytes.as_slice())?;
    if !reply.ok {
        return Err(reply
            .error
            .unwrap_or_else(|| "the admitter refused".into())
            .into());
    }
    let secret = reply
        .group_secret
        .as_deref()
        .and_then(|text| Dgid::parse(text).map(|d| *d.as_bytes()))
        .ok_or("the admitter sent no usable group secret")?;
    let group = Identity::from_secret(secret);
    if group.dgid() != invite.dgid {
        return Err("the group secret does not match the invited dgid".into());
    }
    identity::store(store_dir, &group)?;
    let mut trust = spirit_core::Trust::load(store_dir);
    trust.set(invite.dgid, TrustLevel::Mesh);
    for member in &reply.members {
        if let Some(dgid) = crate::mesh::dgid_of(&member.id.to_string()) {
            trust.set(dgid, TrustLevel::Mesh);
        }
    }
    trust.save(store_dir)?;
    append_seeds(store_dir, &reply.members)?;
    Ok(Joined {
        dgid: invite.dgid,
        head: reply.head.as_deref().and_then(BlobHash::parse),
        members: reply.members,
    })
}

pub async fn join_with_mesh(mesh: &Mesh, invite: &Invite) -> Result<Joined, Box<dyn Error>> {
    let joined = join(mesh.endpoint(), mesh.store_dir(), invite).await?;
    mesh.reload_identity();
    for member in &joined.members {
        let ticket = EndpointTicket::from(member.clone()).to_string();
        let _ = mesh.seed(&ticket);
        mesh.set_trust(&member.id.to_string(), TrustLevel::Mesh);
    }
    mesh.want(device::GROUP_COLLECTION);
    Ok(joined)
}

pub fn seeds_path(store_dir: &Path) -> std::path::PathBuf {
    store_dir.join("seeds")
}

pub fn read_seeds(store_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(seeds_path(store_dir))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

fn append_seeds(store_dir: &Path, members: &[EndpointAddr]) -> std::io::Result<()> {
    let mut lines = read_seeds(store_dir);
    for member in members {
        let ticket = EndpointTicket::from(member.clone()).to_string();
        if !lines
            .iter()
            .any(|line| line.contains(&member.id.to_string()) || *line == ticket)
        {
            lines.push(ticket);
        }
    }
    std::fs::create_dir_all(store_dir)?;
    std::fs::write(seeds_path(store_dir), lines.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invite_round_trips_through_its_url() {
        let invite = Invite {
            ticket: "endpointabc".into(),
            dgid: Identity::from_secret([3; 32]).dgid(),
            token: [7; 32],
        };
        let url = invite.url();
        assert!(url.starts_with("spirit://pair?ticket=endpointabc&dgid="));
        assert_eq!(Invite::parse(&url).unwrap(), invite);
        assert!(Invite::parse("https://example.com").is_err());
        assert!(Invite::parse("spirit://pair?ticket=x&dgid=zz&token=00").is_err());
    }

    #[test]
    fn an_offer_expires_and_compares_in_constant_time() {
        let offer = Offer::new().unwrap();
        assert!(offer.live());
        assert!(offer.seconds_left() <= OFFER_TTL.as_secs());
        assert_eq!(offer.token_hex().len(), 64);
        assert!(constant_time_eq(&offer.token, &offer.token));
        assert!(!constant_time_eq(&offer.token, &[0; 32]));
        assert!(!constant_time_eq(&offer.token, &offer.token[..31]));
    }

    #[test]
    fn seeds_accumulate_without_duplicates() {
        let dir = std::env::temp_dir().join(format!("spirit-pair-seeds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let addr = EndpointAddr::new(iroh::SecretKey::from_bytes(&[5; 32]).public());
        append_seeds(&dir, std::slice::from_ref(&addr)).unwrap();
        append_seeds(&dir, &[addr]).unwrap();
        assert_eq!(read_seeds(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("spirit-pair-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_device_pairs_into_a_group_end_to_end() {
        let a = scratch("admitter");
        let b = scratch("joiner");
        let serving =
            tokio::time::timeout(Duration::from_secs(60), crate::serve_mesh(&a, &[], &[]))
                .await
                .expect("the admitter binds in time")
                .unwrap();
        let group = identity::load(&a).unwrap().dgid();
        assert_eq!(serving.mesh.self_dgid(), Some(group));
        assert_eq!(serving.mesh.members().len(), 1);

        let invite = serving.mesh.offer_pairing().unwrap();
        assert_eq!(invite.dgid, group);
        let before = identity::load_or_create(&b).unwrap().dgid();
        assert_ne!(before, group);
        let secret = crate::node_secret(&b).unwrap();
        let endpoint = Endpoint::builder(iroh::endpoint::presets::N0)
            .secret_key(secret)
            .bind()
            .await
            .unwrap();
        crate::wait_online(&endpoint).await;
        let joined = tokio::time::timeout(Duration::from_secs(60), join(&endpoint, &b, &invite))
            .await
            .expect("the join completes in time")
            .unwrap();
        assert_eq!(joined.dgid, group);
        assert_eq!(identity::load(&b).unwrap().dgid(), group);
        let b_node = hex(endpoint.id().as_bytes());
        assert!(serving.mesh.members().contains(&b_node));
        assert_eq!(serving.mesh.trust_level(&b_node), TrustLevel::Mesh);
        assert_eq!(spirit_core::Trust::load(&b).level(group), TrustLevel::Mesh);
        assert!(!read_seeds(&b).is_empty());
        assert!(serving.mesh.current_offer().is_none());
        let store_a = a.clone();
        let status = tokio::task::spawn_blocking(move || {
            crate::api::call(
                &store_a,
                &crate::api::ApiRequest {
                    method: "GET".into(),
                    path: "/gateway/status".into(),
                    query: String::new(),
                    body: Vec::new(),
                },
            )
            .map_err(|e| e.to_string())
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(status.status, 200);
        let store_a = a.clone();
        let offered = tokio::task::spawn_blocking(move || {
            crate::api::call(
                &store_a,
                &crate::api::ApiRequest {
                    method: "POST".into(),
                    path: "/gateway/pair".into(),
                    query: String::new(),
                    body: b"{}".to_vec(),
                },
            )
            .map_err(|e| e.to_string())
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            offered.status,
            200,
            "{}",
            String::from_utf8_lossy(&offered.body)
        );
        assert!(String::from_utf8_lossy(&offered.body).contains("spirit://pair?"));
        assert!(crate::lock::holder(&a).is_some());
        let replay = tokio::time::timeout(Duration::from_secs(60), join(&endpoint, &b, &invite))
            .await
            .expect("the replay answers in time");
        assert!(replay.is_err());
        endpoint.close().await;
        serving.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
