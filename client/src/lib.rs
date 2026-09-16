//! A single, ergonomic entry point for spirit: spin up (or attach to) a
//! daemon, then mint records, store bytes, edit collections, resolve
//! identities, and manage peers/trust — all through one [`Client`].
//!
//! This crate adds no new behaviour. Every method is a thin, typed wrapper
//! over what already exists: local store operations open
//! [`spirit_node::ops::Local`] fresh on every call — the exact struct the
//! CLI and the HTTP gateway already share, and re-opened rather than cached
//! for the same reason the gateway does it that way: an identity can change
//! on disk out from under a long-lived process (a pairing join rewrites the
//! group key), so nothing here holds one across calls. Network operations
//! go through [`spirit_node::mesh::Mesh`] when embedded, or through
//! [`spirit_node::ops::daemon_call`] (the same socket-then-gateway dispatch
//! the CLI uses) when attached to a daemon running elsewhere.
//!
//! ```no_run
//! # async fn example() -> Result<(), spirit_client::ClientError> {
//! use spirit_client::{Client, Mode};
//!
//! let client = Client::open("/tmp/my-store", Mode::embedded()).await?;
//! let ci = client.mint_cir("song", &serde_json::json!({ "title": "Leaves from the Vine" }))?;
//! let blob = client.put_blob(b"pretend this is flac bytes")?;
//! let td = client.mint_tdr("flac-encode", &serde_json::json!({ "variant": "flac" }))?;
//! client.share("favorites", ci, td, blob, Some("Leaves from the Vine".into()))?;
//! client.add_peer("endpoint...").await?;
//! client.shutdown().await?;
//! # Ok(())
//! # }
//! ```

mod collection;
mod error;

pub use collection::Collection;
pub use error::ClientError;

pub use spirit_core::record::ClaimKind as RelationKind;
pub use spirit_core::{AttHash, BlobHash, CiHash, Dgid, TdHash, TrustLevel};
pub use spirit_node::ops::{
    ArtifactView, CollectionSummary, CollectionView, IdentityInfo, IndexView, MemberView, OpView,
    RecordView, TrustEntry,
};

use serde::Serialize;
use serde_json::json;
use spirit_node::ops::Local;
use spirit_node::pair::Invite;
use spirit_node::Serving;
use std::path::{Path, PathBuf};

/// How a [`Client`] relates to the daemon for its store.
#[derive(Debug, Clone)]
pub enum Mode {
    /// Data operations only: open the store, start no daemon, attempt no
    /// attach. Every network method returns [`ClientError::NoDaemon`]. Use
    /// this for a one-off script that just needs to read or write records.
    Local,
    /// Bind an iroh endpoint and start the daemon in this process. Use this
    /// when the calling app *is* the peer — a desktop app, a phone that
    /// wants to be a real mesh member, a script that just needs a store to
    /// talk to and doesn't care who runs it.
    Embedded {
        /// Peers to seed on start (endpoint tickets, blob tickets, bare node
        /// ids, or `http(s)://` gateway URLs — the same shapes `--seed`
        /// takes).
        seeds: Vec<String>,
        /// Also serve the HTTP gateway on this port.
        gateway: Option<u16>,
    },
    /// Attach to a daemon already running for this store directory (checked
    /// over its local Unix socket). Data operations still go straight to
    /// the store files; peer/pairing operations are proxied to the running
    /// daemon. Fails with [`ClientError::NotRunning`] if none answers.
    Attach,
    /// Attach if a daemon already answers for this store, otherwise spin
    /// one up embedded. The friendly default for "just make it work".
    AttachOrEmbed { seeds: Vec<String> },
}

impl Mode {
    /// `Embedded` with no seeds and no gateway — the simplest way to become
    /// a peer.
    pub fn embedded() -> Self {
        Mode::Embedded {
            seeds: Vec::new(),
            gateway: None,
        }
    }

    /// `AttachOrEmbed` with no seeds.
    pub fn attach_or_embed() -> Self {
        Mode::AttachOrEmbed { seeds: Vec::new() }
    }
}

/// One peer this client's daemon knows about, and the trust level it
/// resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub id: String,
    pub trust: TrustLevel,
}

/// A one-use pairing code, ready to display as a link or a QR code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingOffer {
    pub url: String,
    pub seconds_left: u64,
}

/// The resolver's pick for a content identity: the winning blob, who
/// attested it, and whether we already hold the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub blob: BlobHash,
    pub signer: Dgid,
    pub held: bool,
}

enum Network {
    Embedded(Serving),
    Attached,
}

/// The client. Local data operations (`put_blob`, `mint_cir`, `collection`,
/// ...) always work, regardless of `Mode` — spirit's store is just files on
/// disk, shared by convention between the CLI, a daemon, and this client.
/// Network operations (`add_peer`, `peers`, `offer_pairing`, `join`, `want`)
/// need a live daemon and return [`ClientError::NoDaemon`] if opened with
/// [`Mode::Local`] or an unsuccessful [`Mode::Attach`].
pub struct Client {
    dir: PathBuf,
    network: Option<Network>,
}

impl Client {
    /// Open a client for the store at `store_dir`, in the given `mode`. The
    /// directory is created if it does not exist, and an identity is
    /// created on first use — same as every other spirit entry point.
    pub async fn open(store_dir: impl Into<PathBuf>, mode: Mode) -> Result<Self, ClientError> {
        let dir = store_dir.into();
        // Touch the store once up front so a bad path fails here rather
        // than on the first real operation.
        {
            let dir = dir.clone();
            blocking(move || Local::open(&dir).map_err(ClientError::store)).await?;
        }
        let network = match mode {
            Mode::Local => None,
            Mode::Embedded { seeds, gateway } => Some(Network::Embedded(
                start_embedded(&dir, &seeds, gateway).await?,
            )),
            Mode::Attach => {
                if !is_alive(&dir).await {
                    return Err(ClientError::NotRunning(dir));
                }
                Some(Network::Attached)
            }
            Mode::AttachOrEmbed { seeds } => {
                if is_alive(&dir).await {
                    Some(Network::Attached)
                } else {
                    Some(Network::Embedded(start_embedded(&dir, &seeds, None).await?))
                }
            }
        };
        Ok(Self { dir, network })
    }

    /// Shut down the daemon this client started. A no-op if this client
    /// attached to a daemon it does not own, or was opened with
    /// [`Mode::Local`].
    pub async fn shutdown(self) -> Result<(), ClientError> {
        if let Some(Network::Embedded(serving)) = self.network {
            serving.shutdown().await.map_err(ClientError::network)?;
        }
        Ok(())
    }

    pub fn store_dir(&self) -> &Path {
        &self.dir
    }

    fn local(&self) -> Result<Local, ClientError> {
        Local::open(&self.dir).map_err(ClientError::store)
    }

    /// This store's group identity (the DGID every attestation and
    /// collection op here signs as). Re-read from disk every call, since a
    /// pairing join can change it underneath a long-lived `Client`.
    pub fn dgid(&self) -> Result<Dgid, ClientError> {
        Ok(self.local()?.identity.dgid())
    }

    /// This process's device/node id, if it is running an embedded daemon.
    /// `None` when attached to a daemon running elsewhere, or opened with
    /// [`Mode::Local`] — ask that daemon directly (e.g. via [`Client::peers`])
    /// if you need its id.
    pub fn node_id(&self) -> Option<&str> {
        match &self.network {
            Some(Network::Embedded(serving)) => Some(&serving.node_id),
            _ => None,
        }
    }

    /// This embedded daemon's dialing ticket — what you'd show as a QR code
    /// or hand to another device for [`Client::add_peer`]. `None` unless
    /// embedded.
    pub fn ticket(&self) -> Option<&str> {
        match &self.network {
            Some(Network::Embedded(serving)) => Some(&serving.ticket),
            _ => None,
        }
    }

    pub fn identity_info(&self) -> Result<IdentityInfo, ClientError> {
        Ok(self.local()?.identity_info())
    }

    // ---------------------------------------------------------------
    // Data — always local, always available.
    // ---------------------------------------------------------------

    pub fn put_blob(&self, bytes: &[u8]) -> Result<BlobHash, ClientError> {
        self.local()?.store.put(bytes).map_err(ClientError::store)
    }

    pub fn get_blob(&self, hash: BlobHash) -> Result<Vec<u8>, ClientError> {
        self.local()?.store.get(hash).map_err(ClientError::store)
    }

    pub fn has_blob(&self, hash: BlobHash) -> Result<bool, ClientError> {
        Ok(self.local()?.store.has(hash))
    }

    pub fn mint_cir(&self, kind: &str, body: &impl Serialize) -> Result<CiHash, ClientError> {
        let value = serde_json::to_value(body).map_err(|e| ClientError::Invalid(e.to_string()))?;
        self.local()?
            .mint_cir(kind, &value)
            .map_err(ClientError::store)
    }

    pub fn mint_tdr(&self, kind: &str, body: &impl Serialize) -> Result<TdHash, ClientError> {
        let value = serde_json::to_value(body).map_err(|e| ClientError::Invalid(e.to_string()))?;
        self.local()?
            .mint_tdr(kind, &value)
            .map_err(ClientError::store)
    }

    pub fn attest_content(
        &self,
        ci: CiHash,
        td: TdHash,
        blob: BlobHash,
    ) -> Result<AttHash, ClientError> {
        self.local()?
            .attest_content(ci, td, blob)
            .map_err(ClientError::store)
    }

    pub fn attest_relation(
        &self,
        kind: RelationKind,
        ci: CiHash,
        other: CiHash,
    ) -> Result<AttHash, ClientError> {
        let label = match kind {
            RelationKind::SameAs => "same-as",
            RelationKind::SupersededBy => "superseded-by",
            RelationKind::PreviousVersion => "previous-version",
            RelationKind::Content => {
                return Err(ClientError::Invalid(
                    "Content is a content attestation, not a relation; use attest_content".into(),
                ))
            }
        };
        self.local()?
            .attest_relation(label, ci, other)
            .map_err(ClientError::store)
    }

    pub fn record(&self, hash: BlobHash) -> Result<RecordView, ClientError> {
        self.local()?.record(hash).map_err(ClientError::store)
    }

    /// Attest `(ci, td) -> blob` and put both the item and the attestation
    /// into `collection_name`, in one call.
    ///
    /// This is the one to reach for when "share this" is the goal: minting
    /// an attestation alone does not make it resolvable to anyone, because
    /// a collection head only replicates and indexes the record hashes it
    /// explicitly carries (`attest_content` on its own — the lower-level
    /// method above — is easy to reach for and silently produces an
    /// attestation nobody will ever see). `share` does both halves —
    /// `collection(name).add(..)` *and* `.attest(..)` — so the result is
    /// actually discoverable once the collection replicates.
    pub fn share(
        &self,
        collection_name: &str,
        ci: CiHash,
        td: TdHash,
        blob: BlobHash,
        label: Option<String>,
    ) -> Result<AttHash, ClientError> {
        let att = self.attest_content(ci, td, blob)?;
        let collection = self.collection(collection_name)?;
        collection.add(ci, label, Some(td))?;
        collection.attest(att.hash())?;
        Ok(att)
    }

    pub fn collections(&self) -> Result<Vec<CollectionSummary>, ClientError> {
        Ok(self.local()?.collections())
    }

    /// A handle for editing or inspecting one named collection.
    pub fn collection(&self, name: &str) -> Result<Collection, ClientError> {
        // Fail fast on a bad path/store, same as every other data method.
        self.local()?;
        Ok(Collection::new(&self.dir, name))
    }

    pub fn index(&self) -> Result<IndexView, ClientError> {
        Ok(self.local()?.index())
    }

    pub fn artifacts(
        &self,
        ci: CiHash,
        minimum: TrustLevel,
        prefer_td: Option<TdHash>,
    ) -> Result<ArtifactView, ClientError> {
        Ok(self.local()?.artifacts(ci, minimum, prefer_td))
    }

    /// The resolver's single pick for a CI, at or above `minimum` trust.
    pub fn resolve(
        &self,
        ci: CiHash,
        minimum: TrustLevel,
    ) -> Result<Option<Resolved>, ClientError> {
        let view = self.artifacts(ci, minimum, None)?;
        Ok(view.pick.map(|pick| Resolved {
            blob: BlobHash::parse(&pick.blob).expect("a pick always names a valid blob hash"),
            signer: Dgid::parse(&pick.signer).expect("a pick always names a valid dgid"),
            held: pick.held,
        }))
    }

    /// [`Client::resolve`] plus fetching the bytes, in one call.
    pub fn resolve_bytes(
        &self,
        ci: CiHash,
        minimum: TrustLevel,
    ) -> Result<Option<Vec<u8>>, ClientError> {
        match self.resolve(ci, minimum)? {
            Some(resolved) => Ok(Some(self.get_blob(resolved.blob)?)),
            None => Ok(None),
        }
    }

    pub fn trust_entries(&self) -> Result<Vec<TrustEntry>, ClientError> {
        Ok(self.local()?.trust_entries())
    }

    pub fn set_trust(&self, who: &str, level: TrustLevel) -> Result<Dgid, ClientError> {
        self.local()?
            .set_trust(who, level)
            .map_err(ClientError::store)
    }

    pub fn members(&self) -> Result<Vec<MemberView>, ClientError> {
        Ok(self.local()?.members())
    }

    pub fn revoke(&self, node_hex: &str) -> Result<Option<BlobHash>, ClientError> {
        let head = self.local()?.revoke(node_hex).map_err(ClientError::store)?;
        if let Some(Network::Embedded(serving)) = &self.network {
            serving.mesh.refresh_membership();
        }
        Ok(head)
    }

    // ---------------------------------------------------------------
    // Network — needs a live daemon, embedded or attached.
    // ---------------------------------------------------------------

    /// Add a peer by endpoint ticket, blob ticket, bare node id, or
    /// `http(s)://` gateway URL — the same shapes `spirit-node`'s `--seed`
    /// flag takes. Returns the peer's node id. Seeding grants `cache`
    /// trust, so this alone is enough for the peer's advertised collections
    /// to start replicating once you also call [`Client::want`].
    pub async fn add_peer(&self, seed: &str) -> Result<String, ClientError> {
        match self.network()? {
            Network::Embedded(serving) => {
                let mesh = serving.mesh.clone();
                let seed = seed.to_string();
                blocking(move || mesh.seed(&seed).map_err(ClientError::network)).await
            }
            Network::Attached => {
                let reply = self
                    .call_daemon("POST", "/gateway/seed", json!({ "value": seed }))
                    .await?;
                reply
                    .get("node_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .ok_or_else(|| ClientError::Network("daemon reply had no node_id".into()))
            }
        }
    }

    /// Mark a collection as wanted: pull it (and keep pulling updates to
    /// it) from any peer trusted at `cache` or above that advertises it.
    /// Embedded-only today — there is no remote hook for this yet, so this
    /// returns [`ClientError::Network`] when attached.
    pub async fn want(&self, collection_name: &str) -> Result<(), ClientError> {
        match self.network()? {
            Network::Embedded(serving) => {
                serving.mesh.want(collection_name);
                Ok(())
            }
            Network::Attached => Err(ClientError::Network(
                "want is only supported on an embedded client today".into(),
            )),
        }
    }

    /// Every peer this daemon knows about, with the trust level it resolves
    /// to.
    pub async fn peers(&self) -> Result<Vec<PeerInfo>, ClientError> {
        match self.network()? {
            Network::Embedded(serving) => {
                let mesh = serving.mesh.clone();
                blocking(move || {
                    Ok(mesh
                        .trusted_peers()
                        .into_iter()
                        .map(|(id, trust)| PeerInfo { id, trust })
                        .collect())
                })
                .await
            }
            Network::Attached => {
                let stats = self
                    .call_daemon("GET", "/gateway/stats", json!(null))
                    .await?;
                let trusted = stats
                    .get("peers")
                    .and_then(|p| p.get("trusted"))
                    .and_then(|t| t.as_array())
                    .cloned()
                    .unwrap_or_default();
                Ok(trusted
                    .into_iter()
                    .filter_map(|entry| {
                        let id = entry.get("id")?.as_str()?.to_string();
                        let trust = TrustLevel::parse(entry.get("level")?.as_str()?)?;
                        Some(PeerInfo { id, trust })
                    })
                    .collect())
            }
        }
    }

    /// Offer a one-use, ten-minute pairing code for a second device to join
    /// this store's device group.
    pub async fn offer_pairing(&self) -> Result<PairingOffer, ClientError> {
        match self.network()? {
            Network::Embedded(serving) => {
                let mesh = serving.mesh.clone();
                blocking(move || {
                    let invite = mesh.offer_pairing().ok_or_else(|| {
                        ClientError::Network("this store holds no group key".into())
                    })?;
                    Ok(PairingOffer {
                        url: invite.url(),
                        seconds_left: pair_ttl_secs(),
                    })
                })
                .await
            }
            Network::Attached => {
                let reply = self.call_daemon("POST", "/gateway/pair", json!({})).await?;
                let url = reply
                    .get("url")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ClientError::Network("daemon reply had no url".into()))?
                    .to_string();
                let seconds_left = reply
                    .get("seconds_left")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                Ok(PairingOffer { url, seconds_left })
            }
        }
    }

    /// Join the group that offered a pairing link (from
    /// [`Client::offer_pairing`] or `spirit-node pair` on another device).
    /// This device's store adopts the offering group's key.
    pub async fn join(&self, pairing_url: &str) -> Result<(), ClientError> {
        let invite = Invite::parse(pairing_url).map_err(ClientError::Invalid)?;
        match self.network()? {
            Network::Embedded(serving) => {
                spirit_node::pair::join_with_mesh(&serving.mesh, &invite)
                    .await
                    .map_err(ClientError::network)?;
                Ok(())
            }
            Network::Attached => {
                self.call_daemon("POST", "/gateway/join", json!({ "url": pairing_url }))
                    .await?;
                Ok(())
            }
        }
    }

    fn network(&self) -> Result<&Network, ClientError> {
        self.network.as_ref().ok_or(ClientError::NoDaemon)
    }

    async fn call_daemon(
        &self,
        method: &'static str,
        path: &'static str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let dir = self.dir.clone();
        blocking(move || {
            let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
            let (status, reply_bytes) =
                spirit_node::ops::daemon_call(&dir, method, path, &body_bytes)
                    .ok_or_else(|| ClientError::NotRunning(dir.clone()))?
                    .map_err(ClientError::network)?;
            let value: serde_json::Value =
                serde_json::from_slice(&reply_bytes).unwrap_or(serde_json::Value::Null);
            if !(200..300).contains(&status) {
                let message = value
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("daemon call failed")
                    .to_string();
                return Err(ClientError::Network(message));
            }
            Ok(value)
        })
        .await
    }
}

/// Pairing offers are valid for ten minutes; kept in one place so the
/// embedded branch (which only has the invite, not the offer's own TTL
/// bookkeeping) reports the same number the gateway does.
fn pair_ttl_secs() -> u64 {
    spirit_node::pair::OFFER_TTL.as_secs()
}

async fn is_alive(dir: &Path) -> bool {
    let dir = dir.to_path_buf();
    blocking(move || Ok::<_, ClientError>(spirit_node::api::alive(&dir)))
        .await
        .unwrap_or(false)
}

async fn start_embedded(
    dir: &Path,
    seeds: &[String],
    gateway: Option<u16>,
) -> Result<Serving, ClientError> {
    let serving = spirit_node::serve_mesh(dir, seeds, &[])
        .await
        .map_err(ClientError::network)?;
    if let Some(port) = gateway {
        spirit_node::gateway::spawn(
            port,
            spirit_node::gateway::Gateway {
                dir: dir.to_path_buf(),
                node_id: serving.node_id.clone(),
                mesh: serving.mesh.clone(),
                resolvers: spirit_node::gateway::Resolvers::new(),
            },
        )
        .await
        .map_err(ClientError::network)?;
    }
    Ok(serving)
}

/// Run a blocking closure on a blocking-friendly thread when a Tokio
/// runtime is available, or inline otherwise. Every underlying spirit call
/// this wraps — `Local::open`, `Mesh::seed`, the local-socket/gateway RPC in
/// `ops::daemon_call` — is genuinely blocking I/O, exactly like
/// `spirit_node::fetch_blocking` wraps `fetch` for callers with no async
/// runtime of their own; this is that same idiom applied uniformly.
async fn blocking<T, E, F>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<tokio::task::JoinError> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.spawn_blocking(f).await?,
        Err(_) => f(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spirit_core::record::ClaimKind;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("spirit-client-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn attach_fails_fast_when_nothing_is_listening() {
        let dir = scratch("attach-none");
        let result = Client::open(&dir, Mode::Attach).await;
        assert!(matches!(result, Err(ClientError::NotRunning(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn data_operations_need_no_daemon_at_all() {
        let dir = scratch("no-daemon-data");
        let client = Client::open(&dir, Mode::Local).await.unwrap();
        let ci = client
            .mint_cir("song", &json!({ "title": "Leaves from the Vine" }))
            .unwrap();
        let blob = client.put_blob(b"pretend flac bytes").unwrap();
        let td = client
            .mint_tdr("flac-encode", &json!({ "variant": "flac" }))
            .unwrap();
        client
            .share(
                "favorites",
                ci,
                td,
                blob,
                Some("Leaves from the Vine".into()),
            )
            .unwrap();
        let resolved = client.resolve(ci, TrustLevel::Cache).unwrap().unwrap();
        assert_eq!(resolved.blob, blob);
        assert_eq!(
            client.resolve_bytes(ci, TrustLevel::Cache).unwrap(),
            Some(b"pretend flac bytes".to_vec())
        );
        assert_eq!(client.node_id(), None);
        assert_eq!(client.ticket(), None);
        assert!(matches!(
            client.add_peer("endpoint...").await,
            Err(ClientError::NoDaemon)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_embedded_client_seeds_and_pairs_a_second_one() {
        let a_dir = scratch("embedded-a");
        let b_dir = scratch("embedded-b");
        let a = Client::open(&a_dir, Mode::embedded()).await.unwrap();
        let b = Client::open(&b_dir, Mode::embedded()).await.unwrap();

        let offer = a.offer_pairing().await.unwrap();
        assert!(offer.seconds_left > 0);
        b.join(&offer.url).await.unwrap();
        assert_eq!(b.dgid().unwrap(), a.dgid().unwrap());

        // b's ticket, seeded onto itself through a's remote handle, exercises
        // the plain add_peer/seed path end to end (not just pairing).
        let b_ticket = b.ticket().unwrap().to_string();
        let added = a.add_peer(&b_ticket).await.unwrap();
        assert_eq!(added, b.node_id().unwrap());

        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
    }

    #[tokio::test]
    async fn attaching_reaches_an_already_running_daemon() {
        let dir = scratch("attach-real");
        let daemon = Client::open(&dir, Mode::embedded()).await.unwrap();

        let attached = Client::open(&dir, Mode::Attach).await.unwrap();
        assert_eq!(attached.node_id(), None); // attach doesn't expose the daemon's id locally
        assert_eq!(attached.dgid().unwrap(), daemon.dgid().unwrap());

        // Data written through the attached handle is immediately visible
        // to the embedded one — they're just two views of the same files.
        let ci = attached
            .mint_cir("item", &json!({ "name": "shared" }))
            .unwrap();
        let view = daemon.record(ci.hash()).unwrap();
        assert_eq!(view.kind, "cir (item)");

        // A self-seed round trip through the daemon's own Mesh, proving the
        // /gateway/seed route this crate added is reachable via Attach.
        let own_ticket = daemon.ticket().unwrap().to_string();
        let seeded = attached.add_peer(&own_ticket).await.unwrap();
        assert_eq!(seeded, daemon.node_id().unwrap());

        attached.shutdown().await.unwrap(); // no-op: attach never owns the daemon
        daemon.shutdown().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_relation_kind_that_is_not_a_relation_is_refused() {
        let dir = scratch("bad-relation");
        let client = Client::open(&dir, Mode::Local).await.unwrap();
        let a = client.mint_cir("item", &json!({"name": "a"})).unwrap();
        let b = client.mint_cir("item", &json!({"name": "b"})).unwrap();
        let error = client
            .attest_relation(ClaimKind::Content, a, b)
            .unwrap_err();
        assert!(matches!(error, ClientError::Invalid(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
