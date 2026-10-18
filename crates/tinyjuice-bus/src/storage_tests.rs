use super::*;
use tinystoragedrivers_core::schema::{Item, MigrateOptions, Registry, ScopeClass};
use tinystoragedrivers_core::{MemoryStorage, Scope, StorageBackend};

const LEGACY: &str = "tinyjuice_ccr/";

#[test]
fn the_schema_declares_the_ccr_blobs_and_the_savings_collection_at_module_scope() {
    let schema = storage_schema();
    let items: Vec<_> = schema.items.iter().collect();
    assert_eq!(items.len(), 2, "CCR blobs and savings are all juice stores");
    match items[0] {
        Item::Blobs { prefix, scope } => {
            assert_eq!(prefix, "juice/ccr/");
            assert_eq!(*scope, ScopeClass::Module);
        }
        other => panic!("expected a blob prefix, got {other:?}"),
    }
    match items[1] {
        Item::Collection {
            name,
            version,
            scope,
            ..
        } => {
            assert_eq!(name, "juice.savings");
            assert_eq!(*version, 1);
            assert_eq!(*scope, ScopeClass::Module);
        }
        other => panic!("expected the savings collection, got {other:?}"),
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

#[test]
fn a_key_outside_the_legacy_prefix_has_nothing_to_move() {
    let moved = move_ccr_blob("elsewhere/x".into(), None, b"x".to_vec()).unwrap();
    assert!(moved.is_none());
    let (key, content_type, bytes) = move_ccr_blob(
        "tinyjuice_ccr/ab".into(),
        Some("text/plain".into()),
        b"x".to_vec(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(key, "juice/ccr/ab");
    assert_eq!(content_type.as_deref(), Some("text/plain"));
    assert_eq!(bytes, b"x");
}

#[test]
fn the_savings_body_is_snake_case_with_ms_times_and_round_trips() {
    use crate::savings::{SavingsAggregate, SavingsBucket, StoredSavings};
    let mut aggregate = SavingsAggregate::default();
    let bucket = SavingsBucket {
        events: 1,
        original_tokens: 200,
        compacted_tokens: 100,
        tokens_saved: 100,
        cost_saved_usd: 0.5,
    };
    aggregate.total = bucket.clone();
    aggregate.by_model.insert("m".into(), bucket);
    let body = serde_json::to_value(StoredSavings::from_aggregate(&aggregate, 1234)).unwrap();
    assert_eq!(body["total"]["tokens_saved"], 100);
    assert_eq!(body["by_model"]["m"]["original_tokens"], 200);
    assert_eq!(body["updated_ms"], 1234);
    assert!(body["total"].get("tokensSaved").is_none());
    let back: StoredSavings = serde_json::from_value(body).unwrap();
    assert_eq!(back.into_aggregate().by_model["m"].compacted_tokens, 100);
}
