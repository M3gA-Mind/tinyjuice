use super::*;
use crate::storage::tests::rig;
use tinyjuice_bus::savings::SavingsBucket;
use tinystoragedrivers_core::{Scope, StorageBackend};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn req(original: u64, compacted: u64) -> RecordSavingsRequest {
    RecordSavingsRequest {
        model: "m".into(),
        compressor: "generic".into(),
        original_tokens: original,
        compacted_tokens: compacted,
        input_per_mtok_usd: Some(2.0),
    }
}

async fn stored(
    rig: &crate::storage::tests::Rig,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    let docs = Arc::clone(rig.backend.for_scope(&Scope::local())?.documents());
    Ok(docs
        .get(SAVINGS_COLLECTION, SAVINGS_DOC_ID)
        .await?
        .map(|found| found.doc))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn records_land_as_a_snake_case_document_with_ms_time() -> TestResult {
    let rig = rig(true).await?;
    let savings = Savings::new();
    savings
        .record(Some(&rig.module), &req(2_000_000, 1_000_000))
        .await;
    let body = stored(&rig).await?.ok_or("document written")?;
    assert_eq!(body["total"]["tokens_saved"], 1_000_000);
    assert_eq!(body["by_model"]["m"]["events"], 1);
    assert_eq!(body["by_compressor"]["generic"]["events"], 1);
    assert!(body["updated_ms"].as_u64().unwrap_or(0) > 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_process_resumes_the_stored_totals() -> TestResult {
    let rig = rig(true).await?;
    Savings::new()
        .record(Some(&rig.module), &req(200, 100))
        .await;
    let resumed = Savings::new();
    assert_eq!(
        resumed.stats(Some(&rig.module)).await.total.tokens_saved,
        100
    );
    resumed.record(Some(&rig.module), &req(200, 100)).await;
    assert_eq!(resumed.stats(Some(&rig.module)).await.total.events, 2);
    resumed.reset(Some(&rig.module)).await;
    let body = stored(&rig).await?.ok_or("empty document kept")?;
    assert_eq!(body["total"]["events"], 0);
    assert_eq!(
        Savings::new().stats(Some(&rig.module)).await.total.events,
        0
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn import_adopts_a_legacy_snapshot_only_when_nothing_was_recorded() -> TestResult {
    let rig = rig(true).await?;
    let mut legacy = SavingsAggregate::default();
    legacy.total = SavingsBucket {
        events: 3,
        original_tokens: 30,
        compacted_tokens: 10,
        tokens_saved: 20,
        cost_saved_usd: 0.1,
    };
    let savings = Savings::new();
    assert!(savings.import(Some(&rig.module), legacy.clone()).await);
    assert_eq!(
        savings.stats(Some(&rig.module)).await.total.tokens_saved,
        20
    );
    assert_eq!(stored(&rig).await?.ok_or("written")?["total"]["events"], 3);
    // Importing again must not double count.
    assert!(!savings.import(Some(&rig.module), legacy).await);
    assert_eq!(savings.stats(Some(&rig.module)).await.total.events, 3);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_storage_host_totals_stay_in_memory() -> TestResult {
    let rig = rig(false).await?;
    let savings = Savings::new();
    savings.record(Some(&rig.module), &req(200, 100)).await;
    assert_eq!(savings.stats(Some(&rig.module)).await.total.events, 1);
    assert!(stored(&rig).await?.is_none());
    Ok(())
}
