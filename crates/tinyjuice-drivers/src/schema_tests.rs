use super::*;
use tinystoragedrivers_core::schema::{Item, MigrateOptions, Registry, ScopeClass};
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};

const LEGACY: &str = "tinyjuice_ccr/";

#[test]
fn the_schema_declares_the_ccr_blobs_at_module_scope() {
    let schema = storage_schema();
    let items: Vec<_> = schema.items().iter().collect();
    assert_eq!(items.len(), 1, "CCR blobs are the only thing juice stores");
    match items[0] {
        Item::Blobs { prefix, scope } => {
            assert_eq!(prefix, "juice/ccr/");
            assert_eq!(*scope, ScopeClass::Module);
        }
        other => panic!("expected a blob prefix, got {other:?}"),
    }
    assert!(
        format!("{CCR_PREFIX}/token").starts_with("juice/ccr/"),
        "keys the store writes sit under the declared prefix"
    );
    Registry::compose([schema]).expect("the schema composes");
}

#[tokio::test]
async fn migration_moves_v1_ccr_blobs_under_the_v2_prefix() {
    let backend = MemoryStorage::new();
    let storage = backend.for_scope(&Scope::local()).unwrap();
    let blobs = storage.blobs();
    blobs
        .put(
            "tinyjuice_ccr/0123456789abcdef",
            b"original text".to_vec(),
            Some("text/plain; charset=utf-8"),
        )
        .await
        .unwrap();
    blobs
        .put("tinyjuice_ccr/fedcba9876543210", b"two".to_vec(), None)
        .await
        .unwrap();
    // Not ours: must be left alone.
    blobs
        .put("other/keep", b"keep".to_vec(), None)
        .await
        .unwrap();

    let registry = Registry::compose([storage_schema()]).unwrap();
    let report = registry
        .migrate(&storage, &MigrateOptions::new())
        .await
        .unwrap();
    assert_eq!(report.ran[0].written, 2);

    assert!(blobs.list(LEGACY).await.unwrap().is_empty());
    let moved = blobs
        .get("juice/ccr/0123456789abcdef")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.bytes, b"original text");
    assert_eq!(
        moved.meta.content_type.as_deref(),
        Some("text/plain; charset=utf-8")
    );
    let moved = blobs
        .get("juice/ccr/fedcba9876543210")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(moved.meta.content_type, None);
    assert!(blobs.get("other/keep").await.unwrap().is_some());

    let again = registry
        .migrate(&storage, &MigrateOptions::new())
        .await
        .unwrap();
    assert!(again.ran.is_empty(), "a finished step does not run again");
}

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
    let got = tokio::task::spawn_blocking(move || {
        BlobCcrStore::new(blobs, None).unwrap().get(&token)
    })
    .await
    .unwrap();
    assert_eq!(got.as_deref(), Some("legacy original"));
}
