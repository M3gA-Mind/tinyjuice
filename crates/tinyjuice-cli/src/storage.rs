//! CLI-owned composition of CCR over Tiny Storage Drivers' file backend.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tinyjuice::cache::{CcrStore, set_durable_tier};
use tinyjuice_drivers::{BlobCcrStore, CCR_PREFIX};
use tinystoragedrivers::{Scope, StorageConfig, open};
use tinystoragedrivers_api::BlobStore;
use tinystoragedrivers_types::BlobMeta;

const PAGE_SIZE: usize = 1000;

pub(crate) struct Inventory {
    pub entries: Vec<BlobMeta>,
    pub ignored_entries: usize,
}

pub(crate) fn configure(root: &Path, ttl: Option<Duration>) -> Result<(), String> {
    std::fs::create_dir_all(root)
        .map_err(|error| format!("cannot create CCR storage directory {}: {error}", root.display()))?;
    let blobs = open_file_blobs(root)?;
    import_legacy_files(root, &blobs, ttl)?;
    let store = BlobCcrStore::new(blobs, ttl).map_err(|error| error.to_string())?;
    set_durable_tier(Some(Arc::new(store)));
    Ok(())
}

pub(crate) fn inventory(root: &Path) -> Result<Inventory, String> {
    configure(root, None)?;
    let blobs = open_file_blobs(root)?;
    let mut entries = Vec::new();
    let mut after = None;
    loop {
        let page = run_blob({
            let after = after.clone();
            let blobs = Arc::clone(&blobs);
            async move { blobs.list_page(&format!("{CCR_PREFIX}/"), after.as_deref(), PAGE_SIZE).await }
        })?;
        entries.extend(page.items.into_iter().map(|item| item.meta));
        after = page.next_after;
        if after.is_none() {
            break;
        }
    }
    Ok(Inventory { entries, ignored_entries: 0 })
}

pub(crate) fn collect(
    root: &Path,
    ttl: Option<Duration>,
    max_bytes: Option<u64>,
) -> Result<(usize, u64, usize, u64), String> {
    configure(root, ttl)?;
    let blobs = open_file_blobs(root)?;
    let mut entries = list_all(Arc::clone(&blobs))?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let ttl_ms = ttl.map(|ttl| ttl.as_millis().min(u64::MAX as u128) as u64);
    let mut remove = Vec::new();
    for entry in &entries {
        if ttl_ms.is_some_and(|ttl| {
            entry.written_at_ms.is_some_and(|written| now_ms.saturating_sub(written) >= ttl)
        }) {
            remove.push(entry.meta.key.clone());
        }
    }
    for key in &remove {
        delete_blob(Arc::clone(&blobs), key)?;
    }
    entries.retain(|entry| !remove.contains(&entry.meta.key));
    let mut total = entries.iter().map(|entry| entry.meta.len).sum::<u64>();
    if let Some(budget) = max_bytes {
        entries.sort_by_key(|entry| entry.written_at_ms.unwrap_or(0));
        for entry in entries {
            if total <= budget {
                break;
            }
            delete_blob(Arc::clone(&blobs), &entry.meta.key)?;
            total = total.saturating_sub(entry.meta.len);
            remove.push(entry.meta.key);
        }
    }
    let remaining = list_all(blobs)?;
    let remaining_bytes = remaining.iter().map(|entry| entry.meta.len).sum();
    Ok((remove.len(), remaining_bytes, remaining.len(), remaining_bytes))
}

fn open_file_blobs(root: &Path) -> Result<Arc<dyn BlobStore>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start storage runtime: {error}"))?;
    let url = format!("file:{}", root.display());
    let config = StorageConfig::parse(&url).map_err(|error| error.to_string())?;
    let backend = runtime.block_on(open(&config)).map_err(|error| error.to_string())?;
    let scope = Scope::new("tinyjuice-cli").map_err(|error| error.to_string())?;
    let storage = backend.for_scope(&scope).map_err(|error| error.to_string())?;
    let blobs = Arc::clone(storage.blobs());
    drop(runtime);
    Ok(blobs)
}

fn import_legacy_files(
    root: &Path,
    blobs: &Arc<dyn BlobStore>,
    ttl: Option<Duration>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(root)
        .map_err(|error| format!("failed to inspect legacy CCR directory {}: {error}", root.display()))?;
    let store = BlobCcrStore::new(Arc::clone(blobs), ttl).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to inspect legacy CCR entry: {error}"))?;
        if !entry.file_type().map_err(|error| error.to_string())?.is_file() {
            continue;
        }
        let token = entry.file_name().to_string_lossy().into_owned();
        if !is_token(&token) {
            continue;
        }
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(content) => content,
            Err(error) => return Err(format!("cannot import legacy CCR entry {token}: {error}")),
        };
        let put = store.put(&content);
        if put.token() != token || !put.retained() {
            return Err(format!("legacy CCR entry {token} was not durably retained"));
        }
        std::fs::remove_file(entry.path())
            .map_err(|error| format!("cannot remove imported CCR entry {token}: {error}"))?;
    }
    Ok(())
}

fn list_all(blobs: Arc<dyn BlobStore>) -> Result<Vec<tinystoragedrivers_types::BlobListEntry>, String> {
    let mut entries = Vec::new();
    let mut after = None;
    loop {
        let page = run_blob({
            let after = after.clone();
            let blobs = Arc::clone(&blobs);
            async move { blobs.list_page(&format!("{CCR_PREFIX}/"), after.as_deref(), PAGE_SIZE).await }
        })?;
        entries.extend(page.items);
        after = page.next_after;
        if after.is_none() {
            return Ok(entries);
        }
    }
}

fn delete_blob(blobs: Arc<dyn BlobStore>, key: &str) -> Result<(), String> {
    let key = key.to_owned();
    let handle = Arc::clone(&blobs);
    run_blob(async move { handle.delete(&key).await }).map(|_| ())
}

fn run_blob<T: Send + 'static>(
    future: impl std::future::Future<Output = tinystoragedrivers_types::Result<T>> + Send + 'static,
) -> Result<T, String> {
    // Each helper owns a separate current-thread runtime. Blob engines in this
    // CLI are self-contained and do not depend on work from another runtime.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("cannot start storage runtime: {error}"))?;
    runtime.block_on(future).map_err(|error| error.to_string())
}

fn is_token(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}
