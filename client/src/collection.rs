use crate::ClientError;
use spirit_core::{BlobHash, CiHash, TdHash};
use spirit_node::ops::{CollectionView, Local, OpView};
use std::path::Path;

/// A named collection, scoped to one [`crate::Client`].
///
/// Returned by [`crate::Client::collection`] rather than held by reference:
/// every method here is a thin, synchronous call straight into the store
/// (via [`spirit_node::ops::Local`], the exact struct the CLI and HTTP
/// gateway already share). Like [`crate::Client`] itself, it re-opens the
/// store fresh rather than caching an identity across calls, so it never
/// signs with a group key this store has since moved past (e.g. after a
/// pairing join changed it).
pub struct Collection {
    dir: std::path::PathBuf,
    name: String,
}

impl Collection {
    pub(crate) fn new(dir: &Path, name: &str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            name: name.to_string(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    fn local(&self) -> Result<Local, ClientError> {
        Local::open(&self.dir).map_err(ClientError::store)
    }

    /// Add (or replace, if `ci` is already present) an item.
    pub fn add(
        &self,
        ci: CiHash,
        label: Option<String>,
        default_td: Option<TdHash>,
    ) -> Result<BlobHash, ClientError> {
        self.local()?
            .edit_collection(
                &self.name,
                spirit_node::ops::Edit::Add {
                    ci,
                    label,
                    default_td,
                },
            )
            .map_err(ClientError::store)
    }

    pub fn remove(&self, ci: CiHash) -> Result<BlobHash, ClientError> {
        self.local()?
            .edit_collection(&self.name, spirit_node::ops::Edit::Remove { ci })
            .map_err(ClientError::store)
    }

    /// Carry an attestation's hash in the collection head, so it replicates
    /// alongside the collection's items.
    pub fn attest(&self, hash: BlobHash) -> Result<BlobHash, ClientError> {
        self.local()?
            .edit_collection(&self.name, spirit_node::ops::Edit::Attest { hash })
            .map_err(ClientError::store)
    }

    /// Carry any other record or blob hash in the head's replication closure.
    pub fn record(&self, hash: BlobHash) -> Result<BlobHash, ClientError> {
        self.local()?
            .edit_collection(&self.name, spirit_node::ops::Edit::Record { hash })
            .map_err(ClientError::store)
    }

    /// The folded items, attestations, and closure sizes.
    pub fn show(&self) -> Result<CollectionView, ClientError> {
        self.local()?
            .collection(&self.name)
            .map_err(ClientError::store)
    }

    /// The raw signed op log, in fold order.
    pub fn ops(&self) -> Result<Vec<OpView>, ClientError> {
        self.local()?
            .collection_ops(&self.name)
            .map_err(ClientError::store)
    }
}
