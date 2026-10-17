use crate::{BlobCcrStore, storage_schema};
use tinystoragedrivers_core::schema::{MigrateOptions, Registry};
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};

// The schema shape and the pure step are pinned in tinyjuice-bus's
// storage_tests.rs; this covers the part that needs the store from this crate.

#[tokio::test]
async fn a_migrated_original_is_readable_through_the_store() {
    use tinyjuice::cache::store::{CcrStore, short_hash};
    let backend = MemoryStorage::new();
    let storage = backend.for_scope(&Scope::local()).unwrap();
    let token = short_hash("legacy original");
    storage
        .blobs()
        .put(
            &format!("tinyjuice_ccr/{token}"),
            b"legacy original".to_vec(),
            Some("text/plain; charset=utf-8"),
        )
        .await
        .unwrap();
    Registry::compose([storage_schema()])
        .unwrap()
        .migrate(&storage, &MigrateOptions::new())
        .await
        .unwrap();

    let blobs = std::sync::Arc::clone(storage.blobs());
    let got =
        tokio::task::spawn_blocking(move || BlobCcrStore::new(blobs, None).unwrap().get(&token))
            .await
            .unwrap();
    assert_eq!(got.as_deref(), Some("legacy original"));
}
