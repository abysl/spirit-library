#[cfg(feature = "native")]
pub mod api;
pub mod assets;
#[cfg(feature = "native")]
pub mod blobs;
#[cfg(feature = "native")]
pub mod cli;
#[cfg(feature = "native")]
pub mod fetch;
#[cfg(feature = "native")]
pub mod gateway;
pub mod gossip;
#[cfg(feature = "native")]
pub mod lock;
pub mod mesh;
#[cfg(feature = "native")]
pub mod ops;
#[cfg(feature = "native")]
pub mod pair;
pub mod peers;
mod recorder;
pub mod tables;

pub use iroh;
pub use iroh_blobs;
pub use iroh_tickets;
pub use spirit_core;

use mesh::Mesh;
use std::sync::Arc;

use iroh::{endpoint::presets, Endpoint};
use iroh_blobs::store::mem::MemStore;
#[cfg(feature = "native")]
use iroh_blobs::ticket::BlobTicket;
#[cfg(feature = "native")]
use iroh_blobs::BlobFormat;
use iroh_blobs::{BlobsProtocol, Hash};
use iroh_tickets::endpoint::EndpointTicket;
use serde::{Deserialize, Serialize};
use spirit_core::BlobHash;
#[cfg(feature = "native")]
use spirit_core::BlobStore;
use std::error::Error;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fetched {
    pub name: String,
    pub kind: String,
    pub blobs: usize,
}

pub const ONLINE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn wait_online(endpoint: &Endpoint) {
    let _ = n0_future::time::timeout(ONLINE_WAIT, endpoint.online()).await;
}

pub fn iroh_hash(hash: BlobHash) -> Hash {
    Hash::from_bytes(*hash.as_bytes())
}

pub fn spirit_hash(hash: Hash) -> BlobHash {
    BlobHash::from_bytes(*hash.as_bytes())
}

#[cfg(feature = "native")]
pub async fn fetch(
    ticket: &str,
    dir: &Path,
    name: &str,
    progress: impl FnMut(usize, usize),
) -> Result<Fetched, Box<dyn Error>> {
    let ticket = ticket.trim();
    let parsed: BlobTicket = ticket.parse()?;
    let provider = parsed.addr().id.to_string();
    peers::registry().introduce_provider(&provider, ticket);

    let result = fetch_from(parsed, &provider, dir, name, progress).await;
    match &result {
        Ok(fetched) => peers::registry().set_outcome(
            &provider,
            format!("fetched {} blobs for ref {}", fetched.blobs, fetched.name),
        ),
        Err(error) => peers::registry().set_outcome(&provider, format!("fetch failed: {error}")),
    }
    result
}

#[cfg(feature = "native")]
async fn fetch_from(
    ticket: BlobTicket,
    peer: &str,
    dir: &Path,
    name: &str,
    mut progress: impl FnMut(usize, usize),
) -> Result<Fetched, Box<dyn Error>> {
    let store = BlobStore::open(dir)?;
    let iroh_store = blobs::open_iroh(dir).await?;
    let endpoint = Endpoint::bind(presets::N0).await?;
    let downloader = iroh_store.downloader(&endpoint);
    let provider = ticket.addr().id;

    let manifest_hash = spirit_hash(ticket.hash());
    downloader.download(ticket.hash(), Some(provider)).await?;
    let size = blobs::take(&iroh_store, &store, manifest_hash).await?;
    peers::registry().add_payload_received(peer, size);
    let manifest_bytes = store.get(manifest_hash)?;
    let envelope =
        spirit_core::envelope::of(&manifest_bytes).ok_or("the fetched manifest does not decode")?;
    peers::registry().note_ref(peer, name);

    let total = envelope.refs.len();
    for (index, hash) in envelope.refs.iter().enumerate() {
        if !store.has(*hash) {
            downloader
                .download(iroh_hash(*hash), Some(provider))
                .await?;
            let size = blobs::take(&iroh_store, &store, *hash).await?;
            peers::registry().add_payload_received(peer, size);
        }
        progress(index + 1, total);
    }

    recorder::refresh_reachability(&endpoint).await;
    spirit_core::collection::merge(&store, name, manifest_hash)?;
    Ok(Fetched {
        name: name.to_string(),
        kind: envelope.kind,
        blobs: total,
    })
}

pub struct ServedRef {
    pub name: String,
    pub ticket: String,
}

pub struct Serving {
    pub node_id: String,
    pub ticket: String,
    pub imported: usize,
    pub refs: Vec<ServedRef>,
    pub mesh: Arc<Mesh>,
    pub endpoint: Endpoint,
    pub blobs: iroh_blobs::api::Store,
    router: iroh::protocol::Router,
    #[cfg(feature = "native")]
    lock: Option<lock::StoreLock>,
    #[cfg(feature = "native")]
    socket: Option<std::path::PathBuf>,
}

impl Serving {
    pub async fn shutdown(self) -> Result<(), Box<dyn Error>> {
        self.router.shutdown().await?;
        #[cfg(feature = "native")]
        {
            if let Some(socket) = &self.socket {
                let _ = std::fs::remove_file(socket);
            }
            drop(self.lock);
        }
        Ok(())
    }
}

#[cfg(feature = "native")]
pub fn node_secret(dir: &Path) -> Result<iroh::SecretKey, Box<dyn Error>> {
    let group = spirit_core::identity::load_or_create(dir)?;
    let device = spirit_core::identity::load_or_create_device(dir)?;
    let store = BlobStore::open(dir)?;
    spirit_schema::device::ensure_self(&store, &group, &device)?;
    Ok(iroh::SecretKey::from_bytes(&device.secret_bytes()))
}

#[cfg(feature = "native")]
pub async fn serve(dir: &Path) -> Result<Serving, Box<dyn Error>> {
    serve_mesh(dir, &[], &[]).await
}

#[cfg(feature = "native")]
pub async fn serve_with(
    dir: &Path,
    register: impl FnOnce(iroh::protocol::RouterBuilder) -> iroh::protocol::RouterBuilder,
) -> Result<Serving, Box<dyn Error>> {
    serve_mesh_with(dir, &[], &[], register).await
}

pub const QUIC_PORT_VAR: &str = "SPIRIT_QUIC_PORT";

pub fn quic_port() -> Option<u16> {
    quic_port_from(std::env::var(QUIC_PORT_VAR).ok().as_deref())
}

pub fn quic_port_from(value: Option<&str>) -> Option<u16> {
    value?.trim().parse().ok().filter(|port| *port != 0)
}

#[cfg(feature = "native")]
pub fn bound_builder(port: Option<u16>) -> Result<iroh::endpoint::Builder, Box<dyn Error>> {
    let mut builder = Endpoint::builder(presets::N0);
    if let Some(port) = port {
        builder = builder
            .bind_addr(std::net::SocketAddr::from((
                std::net::Ipv4Addr::UNSPECIFIED,
                port,
            )))?
            .bind_addr(std::net::SocketAddr::from((
                std::net::Ipv6Addr::UNSPECIFIED,
                port,
            )))?;
    }
    Ok(builder)
}

#[cfg(feature = "native")]
pub async fn serve_mesh(
    dir: &Path,
    seeds: &[String],
    wants: &[String],
) -> Result<Serving, Box<dyn Error>> {
    serve_mesh_with(dir, seeds, wants, |router| router).await
}

#[cfg(feature = "native")]
pub async fn serve_mesh_with(
    dir: &Path,
    seeds: &[String],
    wants: &[String],
    register: impl FnOnce(iroh::protocol::RouterBuilder) -> iroh::protocol::RouterBuilder,
) -> Result<Serving, Box<dyn Error>> {
    serve_mesh_mode(dir, seeds, wants, false, register).await
}

#[cfg(feature = "native")]
pub async fn serve_published(dir: &Path, seeds: &[String]) -> Result<Serving, Box<dyn Error>> {
    serve_mesh_mode(dir, seeds, &[], true, |router| router).await
}

#[cfg(feature = "native")]
async fn serve_mesh_mode(
    dir: &Path,
    seeds: &[String],
    wants: &[String],
    published_only: bool,
    register: impl FnOnce(iroh::protocol::RouterBuilder) -> iroh::protocol::RouterBuilder,
) -> Result<Serving, Box<dyn Error>> {
    let store_lock = lock::StoreLock::take(dir)?;
    let store = BlobStore::open(dir)?;
    let iroh_store = blobs::open_iroh(dir).await?;
    let served = blobs::reference_all(&iroh_store, &store).await;
    let imported = served.len();

    let endpoint = bound_builder(quic_port())?
        .secret_key(node_secret(dir)?)
        .bind()
        .await?;
    wait_online(&endpoint).await;
    let node_id = endpoint.id();

    let mut refs = Vec::new();
    let refs_dir = store.root().join("refs");
    if let Ok(entries) = std::fs::read_dir(&refs_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path())?;
            let Some(hash) = BlobHash::parse(&text) else {
                continue;
            };
            let ticket = iroh_blobs::ticket::BlobTicket::new(
                node_id.into(),
                iroh_hash(hash),
                BlobFormat::Raw,
            );
            refs.push(ServedRef {
                name,
                ticket: ticket.to_string(),
            });
        }
    }

    let mesh = Mesh::new(dir, endpoint.clone());
    if published_only {
        mesh.publish_only();
    }
    mesh.note_served(served);
    for seed in seeds {
        mesh.seed(seed)?;
    }
    for seed in pair::read_seeds(dir) {
        if let Err(error) = mesh.seed(&seed) {
            eprintln!(
                "skipping seed {seed:?} from {}: {error}",
                pair::seeds_path(dir).display()
            );
        }
    }
    for want in wants {
        mesh.want(want);
    }

    let blob_api = (*iroh_store).clone();
    let blobs = BlobsProtocol::new(&iroh_store, None);
    let router = register(
        iroh::protocol::Router::builder(endpoint.clone())
            .accept(iroh_blobs::ALPN, recorder::RecordingHandler::new(blobs))
            .accept(
                gossip::ALPN,
                recorder::RecordingHandler::new(gossip::GossipHandler::new(mesh.clone())),
            )
            .accept(
                pair::ALPN,
                recorder::RecordingHandler::new(pair::PairHandler::new(mesh.clone())),
            ),
    )
    .spawn();
    recorder::spawn_reachability_watch(endpoint.clone());
    mesh.publish_status();
    mesh::spawn_loop(
        mesh.clone(),
        endpoint.clone(),
        iroh_store,
        dir.to_path_buf(),
    );
    let service = gateway::Service::new(gateway::Gateway {
        dir: dir.to_path_buf(),
        node_id: node_id.to_string(),
        mesh: mesh.clone(),
        resolvers: gateway::Resolvers::new(),
    });
    let socket = match api::serve(service).await {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!("local api: not served: {error}");
            None
        }
    };
    mesh.set_lock(store_lock);

    Ok(Serving {
        node_id: node_id.to_string(),
        ticket: EndpointTicket::from(endpoint.addr()).to_string(),
        imported,
        refs,
        mesh,
        endpoint,
        blobs: blob_api,
        router,
        lock: None,
        socket,
    })
}

pub async fn serve_in_memory(
    secret: iroh::SecretKey,
    seeds: &[String],
) -> Result<Serving, Box<dyn Error>> {
    serve_in_memory_with(secret, seeds, |router| router).await
}

pub async fn serve_in_memory_with(
    secret: iroh::SecretKey,
    seeds: &[String],
    register: impl FnOnce(iroh::protocol::RouterBuilder) -> iroh::protocol::RouterBuilder,
) -> Result<Serving, Box<dyn Error>> {
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await?;
    wait_online(&endpoint).await;
    let node_id = endpoint.id();

    let mesh = Mesh::new(Path::new("ephemeral"), endpoint.clone());
    mesh.set_replicate(false);
    for seed in seeds {
        mesh.seed(seed)?;
    }

    let store = MemStore::new();
    let blob_api = (*store).clone();
    let blobs = BlobsProtocol::new(&store, None);
    let router = register(
        iroh::protocol::Router::builder(endpoint.clone())
            .accept(iroh_blobs::ALPN, recorder::RecordingHandler::new(blobs))
            .accept(
                gossip::ALPN,
                recorder::RecordingHandler::new(gossip::GossipHandler::new(mesh.clone())),
            ),
    )
    .spawn();
    recorder::spawn_reachability_watch(endpoint.clone());
    mesh.publish_status();
    mesh::spawn_gossip_loop(mesh.clone(), endpoint.clone());

    Ok(Serving {
        node_id: node_id.to_string(),
        ticket: EndpointTicket::from(endpoint.addr()).to_string(),
        imported: 0,
        refs: Vec::new(),
        mesh,
        endpoint,
        blobs: blob_api,
        router,
        #[cfg(feature = "native")]
        lock: None,
        #[cfg(feature = "native")]
        socket: None,
    })
}

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("spirit-node-key-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(feature = "native")]
    #[test]
    fn the_quic_port_comes_from_the_environment_and_ignores_junk() {
        assert_eq!(quic_port_from(None), None);
        assert_eq!(quic_port_from(Some(" 4433 ")), Some(4433));
        assert_eq!(quic_port_from(Some("0")), None);
        assert_eq!(quic_port_from(Some("many")), None);
        assert!(bound_builder(Some(4433)).is_ok());
        assert!(bound_builder(None).is_ok());
    }

    #[test]
    fn node_secret_persists_across_loads() {
        let dir = scratch_dir("persists");
        let first = node_secret(&dir).unwrap();
        let second = node_secret(&dir).unwrap();
        assert_eq!(first.public(), second.public());
        let on_disk = std::fs::read_to_string(dir.join("identity").join("node")).unwrap();
        assert_eq!(on_disk.trim().len(), 64);
        let group = spirit_core::identity::load(&dir).unwrap();
        assert_ne!(group.dgid().as_bytes(), first.public().as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_node_key_is_an_error_not_a_new_identity() {
        let dir = scratch_dir("corrupt");
        std::fs::create_dir_all(dir.join("identity")).unwrap();
        std::fs::write(dir.join("identity").join("node"), "not a key").unwrap();
        assert!(node_secret(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn published_mesh_cannot_be_reenabled_or_queue_peer_downloads() {
        let endpoint = Endpoint::builder(presets::N0).bind().await.unwrap();
        let dir = scratch_dir("published");
        let mesh = Mesh::new(&dir, endpoint.clone());
        mesh.publish_only();
        mesh.set_replicate(true);
        mesh.request_blob(BlobHash::of(b"unapproved"), None);
        assert!(!mesh.replicates());
        assert!(!mesh.accepts_downloads());
        assert!(mesh.wanted_blobs().is_empty());
        endpoint.close().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
