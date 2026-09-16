use crate::{iroh_hash, spirit_hash};
use iroh_blobs::api::blobs::{AddPathOptions, ExportMode, ExportOptions, ImportMode};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::BlobFormat;
use spirit_core::{BlobHash, BlobStore};
use std::error::Error;
use std::path::Path;

pub const IROH_DIR: &str = "blobs";
pub const LEGACY_IROH_DIR: &str = "iroh";

pub async fn open_iroh(dir: &Path) -> Result<FsStore, Box<dyn Error>> {
    let legacy = dir.join(LEGACY_IROH_DIR);
    let current = dir.join(IROH_DIR);
    if legacy.is_dir() && !current.is_dir() {
        std::fs::remove_dir_all(&legacy)?;
    }
    Ok(FsStore::load(current).await?)
}

pub async fn reference(
    iroh: &FsStore,
    store: &BlobStore,
    hash: BlobHash,
) -> Result<(), Box<dyn Error>> {
    let path = store.path_of(hash);
    if !path.is_file() {
        return Err(format!("{hash} is not in the store").into());
    }
    let tag = iroh
        .blobs()
        .add_path_with_opts(AddPathOptions {
            path: path.clone(),
            format: BlobFormat::Raw,
            mode: ImportMode::TryReference,
        })
        .await?;
    if spirit_hash(tag.hash) != hash {
        store.remove(hash)?;
        return Err(format!(
            "{} hashed to {}; the corrupt file was removed",
            path.display(),
            spirit_hash(tag.hash)
        )
        .into());
    }
    Ok(())
}

pub async fn reference_all(iroh: &FsStore, store: &BlobStore) -> Vec<String> {
    let mut served = Vec::new();
    for hash in store.hashes() {
        match reference(iroh, store, hash).await {
            Ok(()) => served.push(hash.to_string()),
            Err(error) => eprintln!("store: skipping {hash}: {error}"),
        }
    }
    served
}

pub async fn take(
    iroh: &iroh_blobs::api::Store,
    store: &BlobStore,
    hash: BlobHash,
) -> Result<u64, Box<dyn Error>> {
    if store.has(hash) {
        return Ok(std::fs::metadata(store.path_of(hash))?.len());
    }
    let target = store.path_of(hash);
    let size = iroh
        .blobs()
        .export_with_opts(ExportOptions {
            hash: iroh_hash(hash),
            mode: ExportMode::TryReference,
            target: target.clone(),
        })
        .await?;
    if let Err(error) = store.get(hash) {
        store.remove(hash)?;
        return Err(format!("exported {hash} did not verify: {error}").into());
    }
    Ok(size)
}
