use super::*;

fn request(model: &str, original: u64, compacted: u64, price: Option<f64>) -> RecordSavingsRequest {
    RecordSavingsRequest {
        model: model.into(),
        compressor: "smartcrusher".into(),
        original_tokens: original,
        compacted_tokens: compacted,
        input_per_mtok_usd: price,
    }
}

#[test]
fn records_and_aggregates_with_host_input_price() {
    let ledger = SavingsLedger::new();
    ledger.record(&request("priced", 2_000_000, 1_000_000, Some(3.0)));
    let stats = ledger.stats();
    assert_eq!(stats.total.events, 1);
    assert_eq!(stats.total.original_tokens, 2_000_000);
    assert_eq!(stats.total.compacted_tokens, 1_000_000);
    assert_eq!(stats.total.tokens_saved, 1_000_000);
    assert_eq!(stats.total.cost_saved_usd, 3.0);
    assert_eq!(stats.by_model["priced"].cost_saved_usd, 3.0);
    assert_eq!(stats.by_compressor["smartcrusher"].events, 1);
}

#[test]
fn unpriced_models_keep_token_totals_without_guessing_dollars() {
    let ledger = SavingsLedger::new();
    ledger.record(&request("turn-model-x", 2000, 1000, None));
    assert_eq!(ledger.stats().by_model["turn-model-x"].tokens_saved, 1000);
    assert_eq!(ledger.stats().total.cost_saved_usd, 0.0);
}

#[test]
fn non_smaller_compactions_do_not_record_savings() {
    let ledger = SavingsLedger::new();
    ledger.record(&request("m", 100, 100, Some(3.0)));
    ledger.record(&request("m", 50, 100, Some(3.0)));
    assert_eq!(ledger.stats().total.events, 0);
}

#[test]
fn malformed_price_is_unpriced_and_keeps_snapshots_serializable() {
    for price in [f64::NAN, f64::INFINITY, -1.0] {
        let ledger = SavingsLedger::new();
        ledger.record(&request("m", 200, 100, Some(price)));
        assert_eq!(ledger.stats().total.cost_saved_usd, 0.0);
        assert!(serde_json::to_string(&ledger.stats()).is_ok());
    }
}

#[test]
fn snapshot_round_trip_and_reset_preserve_existing_dashboard_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state/tokenjuice_savings.json");
    let ledger = SavingsLedger::new();
    ledger.configure(&path);
    ledger.record(&request("m", 2000, 1000, Some(3.0)));
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["total"]["tokensSaved"], 1000);
    assert_eq!(saved["total"]["originalTokens"], 2000);
    assert_eq!(saved["byModel"]["m"]["events"], 1);
    assert_eq!(saved["byCompressor"]["smartcrusher"]["events"], 1);
    assert!(saved["total"].get("tokens_saved").is_none());
    let resumed = SavingsLedger::new();
    resumed.configure(&path);
    assert_eq!(resumed.stats().total.tokens_saved, 1000);
    resumed.reset();
    let empty = SavingsLedger::new();
    empty.configure(&path);
    assert_eq!(empty.stats().total.events, 0);
}

#[test]
fn repeated_configure_does_not_reload_a_stale_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("savings.json");
    let ledger = SavingsLedger::new();
    ledger.configure(&path);
    ledger.record(&request("m", 2000, 1000, None));
    std::fs::write(
        &path,
        serde_json::to_string(&SavingsAggregate::default()).unwrap(),
    )
    .unwrap();
    ledger.configure(&path);
    assert_eq!(ledger.stats().total.tokens_saved, 1000);
}

#[test]
fn missing_or_malformed_snapshots_leave_in_memory_totals_intact() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = SavingsLedger::new();
    ledger.record(&request("m", 2000, 1000, None));
    ledger.configure(&dir.path().join("absent.json"));
    let invalid = dir.path().join("invalid.json");
    std::fs::write(&invalid, "{").unwrap();
    ledger.configure(&invalid);
    assert_eq!(ledger.stats().total.tokens_saved, 1000);
}

#[test]
fn loads_a_pre_migration_openhuman_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("savings.json");
    std::fs::write(&path, r#"{
        "total":{"events":1,"originalTokens":2000,"compactedTokens":1000,"tokensSaved":1000,"costSavedUsd":0.003},
        "byModel":{"claude-sonnet-4-6":{"events":1,"originalTokens":2000,"compactedTokens":1000,"tokensSaved":1000,"costSavedUsd":0.003}},
        "byCompressor":{"smartcrusher":{"events":1,"originalTokens":2000,"compactedTokens":1000,"tokensSaved":1000,"costSavedUsd":0.003}}
    }"#).unwrap();
    let ledger = SavingsLedger::new();
    ledger.configure(&path);
    assert_eq!(ledger.stats().total.tokens_saved, 1000);
    ledger.record(&request("claude-sonnet-4-6", 2000, 1000, Some(3.0)));
    let aggregate = ledger.stats();
    assert_eq!(aggregate.total.events, 2);
    assert_eq!(aggregate.by_model["claude-sonnet-4-6"].tokens_saved, 2000);
    assert!((aggregate.by_compressor["smartcrusher"].cost_saved_usd - 0.006).abs() < 1e-10);
}
