//! The `uniffi` boundary for [`spirit_client`]: one blocking, foreign-safe
//! facade over the async `spirit-client` API, so Kotlin (Android, iOS via
//! Kotlin/Native, or desktop JVM) gets a plain synchronous/`suspend` API
//! with no foreign-executor plumbing to wire up.
//!
//! This mirrors an idiom `spirit-node` already uses for exactly this
//! reason: `fetch` (async) has a `fetch_blocking` twin that spins its own
//! Tokio runtime for callers with none. Every method here does the same
//! thing once, for the whole client API: own a background runtime, and
//! `.block_on(...)` the async `spirit_client::Client` method underneath.
//!
//! No business logic lives here — every method is a one-line call into
//! `spirit-client`, converting typed hashes to/from the `ci:`/`td:`/`att:`/
//! `blob:`-prefixed strings spirit already displays them as everywhere else
//! (the CLI, the gateway JSON), and JSON record bodies to/from a plain
//! `String` since uniffi's Kotlin output has no direct analogue for
//! `serde_json::Value`.

use spirit_client::{Client, ClientError, Mode, RelationKind};
use std::sync::Arc;

uniffi::setup_scaffolding!();

/// Errors crossing the FFI boundary. One variant per [`ClientError`]
/// variant, so a Kotlin `catch` block can still branch on what went wrong
/// rather than parsing a message string.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("store: {0}")]
    Store(String),
    #[error("network: {0}")]
    Network(String),
    #[error("no daemon is running for this store")]
    NoDaemon,
    #[error("no daemon answered for {0}")]
    NotRunning(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<ClientError> for FfiError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Store(message) => FfiError::Store(message),
            ClientError::Network(message) => FfiError::Network(message),
            ClientError::NoDaemon => FfiError::NoDaemon,
            ClientError::NotRunning(dir) => FfiError::NotRunning(dir.display().to_string()),
            ClientError::Invalid(message) => FfiError::Invalid(message),
            other => FfiError::Internal(other.to_string()),
        }
    }
}

fn relation_kind(text: &str) -> Result<RelationKind, FfiError> {
    match text {
        "same-as" => Ok(RelationKind::SameAs),
        "superseded-by" => Ok(RelationKind::SupersededBy),
        "previous-version" => Ok(RelationKind::PreviousVersion),
        other => Err(FfiError::Invalid(format!(
            "{other:?} is not a relation; use same-as, superseded-by or previous-version"
        ))),
    }
}

fn json_body(text: &str) -> Result<serde_json::Value, FfiError> {
    serde_json::from_str(text).map_err(|e| FfiError::Invalid(format!("bad JSON body: {e}")))
}

/// One entry in [`SpiritClient::peers`].
#[derive(uniffi::Record)]
pub struct FfiPeer {
    pub id: String,
    pub trust: String,
}

/// A pairing offer, ready to render as a QR code (`url`) with a countdown
/// (`seconds_left`).
#[derive(uniffi::Record)]
pub struct FfiPairingOffer {
    pub url: String,
    pub seconds_left: u64,
}

/// The resolver's pick for a content identity.
#[derive(uniffi::Record)]
pub struct FfiResolved {
    pub blob: String,
    pub signer: String,
    pub held: bool,
}

/// The daemon/client handle Kotlin holds. One background Tokio runtime per
/// instance, used to block on every `spirit-client` async call.
///
/// `inner` is a `Mutex<Option<Client>>`, not a bare `Client`, because uniffi
/// objects are always reference-counted on the foreign side (Kotlin's
/// `SpiritClient` holds a handle that survives independently of any one
/// method call) — there is no way for an exported method to reliably
/// observe "I am the last owner", even one declared to take `self:
/// Arc<Self>`, so a one-time operation like [`SpiritClient::shutdown`] has
/// to be modelled as "take the value out from behind shared access", not
/// "consume self". Discovered by actually running this against the real
/// library: an earlier version took `self: Arc<Self>` and
/// `Arc::try_unwrap`'d it, which failed every time with "other references
/// live" — uniffi's own generated handle map is always that other
/// reference.
#[derive(uniffi::Object)]
pub struct SpiritClient {
    inner: std::sync::Mutex<Option<Client>>,
    runtime: tokio::runtime::Runtime,
}

impl SpiritClient {
    fn with<T>(&self, f: impl FnOnce(&Client) -> Result<T, ClientError>) -> Result<T, FfiError> {
        let guard = self
            .inner
            .lock()
            .expect("the client mutex is never poisoned");
        let client = guard
            .as_ref()
            .ok_or_else(|| FfiError::Internal("this client was already shut down".into()))?;
        Ok(f(client)?)
    }
}

#[uniffi::export]
impl SpiritClient {
    /// Open a client that starts (or reuses) a daemon embedded in this
    /// process — the common case for a mobile/desktop app that wants to be
    /// a real peer. `seeds` are optional peers to seed on start.
    #[uniffi::constructor]
    pub fn open_embedded(store_dir: String, seeds: Vec<String>) -> Result<Arc<Self>, FfiError> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| FfiError::Internal(format!("starting a runtime: {e}")))?;
        let inner = runtime.block_on(Client::open(
            store_dir,
            Mode::Embedded {
                seeds,
                gateway: None,
            },
        ))?;
        Ok(Arc::new(Self {
            inner: std::sync::Mutex::new(Some(inner)),
            runtime,
        }))
    }

    /// Open a client against a store's data only — no daemon started, no
    /// attach attempted. Every network method returns
    /// [`FfiError::NoDaemon`]. Useful for a one-off import without ever
    /// touching the network.
    #[uniffi::constructor]
    pub fn open_local(store_dir: String) -> Result<Arc<Self>, FfiError> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| FfiError::Internal(format!("starting a runtime: {e}")))?;
        let inner = runtime.block_on(Client::open(store_dir, Mode::Local))?;
        Ok(Arc::new(Self {
            inner: std::sync::Mutex::new(Some(inner)),
            runtime,
        }))
    }

    /// Shut down the daemon this client started, if any. Idempotent: a
    /// second call (or any other method call after this one) returns
    /// [`FfiError::Internal`] rather than panicking.
    pub fn shutdown(&self) -> Result<(), FfiError> {
        let taken = self
            .inner
            .lock()
            .expect("the client mutex is never poisoned")
            .take();
        let Some(client) = taken else {
            return Err(FfiError::Internal(
                "this client was already shut down".into(),
            ));
        };
        self.runtime.block_on(client.shutdown())?;
        Ok(())
    }

    pub fn dgid(&self) -> Result<String, FfiError> {
        Ok(self.with(|c| c.dgid())?.to_string())
    }

    pub fn node_id(&self) -> Option<String> {
        self.with(|c| Ok(c.node_id().map(String::from)))
            .ok()
            .flatten()
    }

    pub fn ticket(&self) -> Option<String> {
        self.with(|c| Ok(c.ticket().map(String::from)))
            .ok()
            .flatten()
    }

    // ---- data ----

    pub fn put_blob(&self, bytes: Vec<u8>) -> Result<String, FfiError> {
        Ok(self.with(|c| c.put_blob(&bytes))?.to_string())
    }

    pub fn get_blob(&self, hash: String) -> Result<Vec<u8>, FfiError> {
        let hash = parse_blob(&hash)?;
        self.with(|c| c.get_blob(hash))
    }

    pub fn mint_cir(&self, kind: String, body_json: String) -> Result<String, FfiError> {
        let body = json_body(&body_json)?;
        Ok(self.with(|c| c.mint_cir(&kind, &body))?.to_string())
    }

    pub fn mint_tdr(&self, kind: String, body_json: String) -> Result<String, FfiError> {
        let body = json_body(&body_json)?;
        Ok(self.with(|c| c.mint_tdr(&kind, &body))?.to_string())
    }

    pub fn attest_content(&self, ci: String, td: String, blob: String) -> Result<String, FfiError> {
        let (ci, td, blob) = (parse_ci(&ci)?, parse_td(&td)?, parse_blob(&blob)?);
        Ok(self.with(|c| c.attest_content(ci, td, blob))?.to_string())
    }

    pub fn attest_relation(
        &self,
        kind: String,
        ci: String,
        other: String,
    ) -> Result<String, FfiError> {
        let kind = relation_kind(&kind)?;
        let (ci, other) = (parse_ci(&ci)?, parse_ci(&other)?);
        Ok(self
            .with(|c| c.attest_relation(kind, ci, other))?
            .to_string())
    }

    /// Mint a content attestation and add the item to `collection_name`, in
    /// one call — the one to reach for when the goal is "share this" (see
    /// [`spirit_client::Client::share`] for why the two halves matter).
    pub fn share(
        &self,
        collection_name: String,
        ci: String,
        td: String,
        blob: String,
        label: Option<String>,
    ) -> Result<String, FfiError> {
        let (ci, td, blob) = (parse_ci(&ci)?, parse_td(&td)?, parse_blob(&blob)?);
        Ok(self
            .with(|c| c.share(&collection_name, ci, td, blob, label))?
            .to_string())
    }

    pub fn collection_add(
        &self,
        name: String,
        ci: String,
        label: Option<String>,
    ) -> Result<String, FfiError> {
        let ci = parse_ci(&ci)?;
        Ok(self
            .with(|c| c.collection(&name)?.add(ci, label, None))?
            .to_string())
    }

    pub fn collection_remove(&self, name: String, ci: String) -> Result<String, FfiError> {
        let ci = parse_ci(&ci)?;
        Ok(self.with(|c| c.collection(&name)?.remove(ci))?.to_string())
    }

    /// The folded collection as JSON (same shape as the HTTP gateway's
    /// `/gateway/collection/<name>` route).
    pub fn collection_show(&self, name: String) -> Result<String, FfiError> {
        let view = self.with(|c| c.collection(&name)?.show())?;
        serde_json::to_string(&view).map_err(|e| FfiError::Internal(e.to_string()))
    }

    pub fn resolve(&self, ci: String, minimum: String) -> Result<Option<FfiResolved>, FfiError> {
        let ci = parse_ci(&ci)?;
        let minimum = parse_trust(&minimum)?;
        Ok(self.with(|c| c.resolve(ci, minimum))?.map(|r| FfiResolved {
            blob: r.blob.to_string(),
            signer: r.signer.to_string(),
            held: r.held,
        }))
    }

    pub fn resolve_bytes(&self, ci: String, minimum: String) -> Result<Option<Vec<u8>>, FfiError> {
        let ci = parse_ci(&ci)?;
        let minimum = parse_trust(&minimum)?;
        self.with(|c| c.resolve_bytes(ci, minimum))
    }

    /// Every content identity spirit knows about, plus links and external
    /// ids, as JSON (same shape as `/gateway/index`).
    pub fn index_json(&self) -> Result<String, FfiError> {
        let view = self.with(|c| c.index())?;
        serde_json::to_string(&view).map_err(|e| FfiError::Internal(e.to_string()))
    }

    pub fn set_trust(&self, who: String, level: String) -> Result<(), FfiError> {
        let level = parse_trust(&level)?;
        self.with(|c| c.set_trust(&who, level))?;
        Ok(())
    }

    // ---- network ----

    pub fn add_peer(&self, seed: String) -> Result<String, FfiError> {
        self.with(|c| self.runtime.block_on(c.add_peer(&seed)))
    }

    pub fn want(&self, collection_name: String) -> Result<(), FfiError> {
        self.with(|c| self.runtime.block_on(c.want(&collection_name)))
    }

    pub fn peers(&self) -> Result<Vec<FfiPeer>, FfiError> {
        Ok(self
            .with(|c| self.runtime.block_on(c.peers()))?
            .into_iter()
            .map(|p| FfiPeer {
                id: p.id,
                trust: p.trust.label().to_string(),
            })
            .collect())
    }

    pub fn offer_pairing(&self) -> Result<FfiPairingOffer, FfiError> {
        let offer = self.with(|c| self.runtime.block_on(c.offer_pairing()))?;
        Ok(FfiPairingOffer {
            url: offer.url,
            seconds_left: offer.seconds_left,
        })
    }

    pub fn join(&self, pairing_url: String) -> Result<(), FfiError> {
        self.with(|c| self.runtime.block_on(c.join(&pairing_url)))
    }
}

fn parse_ci(text: &str) -> Result<spirit_client::CiHash, FfiError> {
    spirit_client::CiHash::parse(text)
        .ok_or_else(|| FfiError::Invalid(format!("{text:?} is not a ci: hash")))
}

fn parse_td(text: &str) -> Result<spirit_client::TdHash, FfiError> {
    spirit_client::TdHash::parse(text)
        .ok_or_else(|| FfiError::Invalid(format!("{text:?} is not a td: hash")))
}

fn parse_blob(text: &str) -> Result<spirit_client::BlobHash, FfiError> {
    let bare = text.strip_prefix("blob:").unwrap_or(text);
    spirit_client::BlobHash::parse(bare)
        .ok_or_else(|| FfiError::Invalid(format!("{text:?} is not a blob hash")))
}

fn parse_trust(text: &str) -> Result<spirit_client::TrustLevel, FfiError> {
    spirit_client::TrustLevel::parse(text).ok_or_else(|| {
        FfiError::Invalid(format!(
            "{text:?} must be one of: unknown, contact, cache, mesh"
        ))
    })
}
