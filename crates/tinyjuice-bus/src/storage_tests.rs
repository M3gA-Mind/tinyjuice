use super::*;

#[test]
fn owner_collections_declare_savings_without_engine_or_migration_policy() {
    assert_eq!(collections(), vec![CollectionSpec::new("juice.savings")]);
    assert_eq!(CCR_PREFIX, "juice/ccr");
    assert_eq!(LEGACY_CCR_PREFIX, "tinyjuice_ccr/");
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
