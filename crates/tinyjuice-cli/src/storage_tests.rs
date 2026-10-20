use super::*;
use tinyjuice::cache::CcrStore;
use tinystoragedrivers_api::BlobStore;
use tinystoragedrivers_testkit::FaultPlan;
use tinystoragedrivers_types::{
    Blob, BlobListPage, BlobMeta, Capabilities, ErrorKind, Result as StorageResult,
};

#[derive(Debug)]
struct FailMetadataWrite {
    inner: Arc<dyn BlobStore>,
    faults: FaultPlan,
}

#[async_trait::async_trait]
impl BlobStore for FailMetadataWrite {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn max_key_len(&self) -> usize {
        self.inner.max_key_len()
    }

    async fn put(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: Option<&str>,
    ) -> StorageResult<BlobMeta> {
        if key.starts_with(CCR_METADATA_PREFIX) {
            self.faults.check("metadata_put")?;
        }
        self.inner.put(key, bytes, content_type).await
    }

    async fn get(&self, key: &str) -> StorageResult<Option<Blob>> {
        self.inner.get(key).await
    }

    async fn get_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> StorageResult<Option<Vec<u8>>> {
        self.inner.get_range(key, range).await
    }

    async fn head(&self, key: &str) -> StorageResult<Option<BlobMeta>> {
        self.inner.head(key).await
    }

    async fn delete(&self, key: &str) -> StorageResult<bool> {
        self.inner.delete(key).await
    }

    async fn list(&self, prefix: &str) -> StorageResult<Vec<BlobMeta>> {
        self.inner.list(prefix).await
    }

    async fn list_page(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> StorageResult<BlobListPage> {
        self.inner.list_page(prefix, after, limit).await
    }
}

#[test]
fn collection_reports_bytes_removed_by_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = open_file_blobs(dir.path()).unwrap();
    let store = BlobCcrStore::new(blobs, None).unwrap();
    assert!(store.put("first").retained());
    assert!(store.put("second").retained());

    assert_eq!(collect(dir.path(), None, Some(0)).unwrap(), (2, 11, 0, 0));
}

#[test]
fn durable_cli_writes_enforce_the_configured_byte_budget() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = open_file_blobs(dir.path()).unwrap();
    let store = CappedCcrStore {
        store: BlobCcrStore::new(Arc::clone(&blobs), None).unwrap(),
        blobs: Arc::clone(&blobs),
        ttl: None,
        max_bytes: Some(9),
    };

    assert!(store.put("first").retained());
    assert!(store.put("second").retained());
    let current = store.put("tri");
    assert!(current.retained());

    let entries = run_blob({
        let blobs = Arc::clone(&blobs);
        async move { blobs.list(&format!("{CCR_PREFIX}/")).await }
    })
    .unwrap();
    assert_eq!(entries.iter().map(|entry| entry.len).sum::<u64>(), 9);
    assert!(store.get(current.token()).is_some());
}

#[test]
fn migration_ttl_uses_the_source_file_age() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(95);
    assert_eq!(
        remaining_ttl(modified, now, Duration::from_secs(10)),
        Some(Duration::from_secs(5))
    );
    assert_eq!(remaining_ttl(modified, now, Duration::from_secs(5)), None);
    assert_eq!(
        remaining_ttl(now + Duration::from_secs(1), now, Duration::from_secs(5)),
        Some(Duration::from_secs(5))
    );
}

#[test]
fn collection_uses_imported_write_time_for_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = open_file_blobs(dir.path()).unwrap();
    let content = "legacy ccr with an old semantic timestamp";
    let store = BlobCcrStore::new(Arc::clone(&blobs), None).unwrap();
    let put = store.put(content);
    assert!(put.retained());
    let old_time = unix_epoch_ms(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
    write_semantic_time(Arc::clone(&blobs), put.token(), old_time, None).unwrap();

    assert_eq!(
        prune(&blobs, Some(Duration::from_secs(1)), None, None).unwrap(),
        (1, content.len() as u64, 0, 0)
    );
    assert!(store.get(put.token()).is_none());
    assert_eq!(
        read_semantic_time(Arc::clone(&blobs), put.token()).unwrap(),
        None
    );
}

#[test]
fn collection_orders_by_imported_write_time_under_the_byte_budget() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = open_file_blobs(dir.path()).unwrap();
    let store = BlobCcrStore::new(Arc::clone(&blobs), None).unwrap();
    let older = "older legacy ccr";
    let newer = "newer legacy ccr body";
    let older_put = store.put(older);
    let newer_put = store.put(newer);
    assert!(older_put.retained() && newer_put.retained());
    write_semantic_time(
        Arc::clone(&blobs),
        older_put.token(),
        unix_epoch_ms(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
        None,
    )
    .unwrap();
    write_semantic_time(
        Arc::clone(&blobs),
        newer_put.token(),
        unix_epoch_ms(SystemTime::UNIX_EPOCH + Duration::from_secs(2)),
        None,
    )
    .unwrap();

    assert_eq!(
        prune(&blobs, None, Some(newer.len() as u64), None).unwrap(),
        (1, older.len() as u64, 1, newer.len() as u64)
    );
    assert!(store.get(older_put.token()).is_none());
    assert_eq!(store.get(newer_put.token()).as_deref(), Some(newer));
}

#[test]
fn gc_removes_import_intents_that_have_no_ccr_body() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = open_file_blobs(dir.path()).unwrap();
    let token = "0123456789abcdef0123456789abcdef";
    write_import_intent(
        Arc::clone(&blobs),
        token,
        unix_epoch_ms(SystemTime::now()),
        None,
    )
    .unwrap();

    assert_eq!(prune(&blobs, None, None, None).unwrap(), (0, 0, 0, 0));
    assert_eq!(read_import_intent(blobs, token).unwrap(), None);
}

#[test]
fn failed_timestamp_refresh_does_not_leave_an_old_age_for_gc() {
    let dir = tempfile::tempdir().unwrap();
    let content = "a normal re-offload of imported content";
    let token = tinyjuice::cache::short_hash(content);
    let inner = open_file_blobs(dir.path()).unwrap();
    let original = BlobCcrStore::new(Arc::clone(&inner), None).unwrap();
    assert!(original.put(content).retained());
    let old_time = unix_epoch_ms(SystemTime::now() - Duration::from_secs(86_400));
    write_semantic_time(Arc::clone(&inner), &token, old_time, None).unwrap();

    let faults = FaultPlan::new();
    faults.fail_operation_nth("metadata_put", 1, ErrorKind::Backend);
    let failing: Arc<dyn BlobStore> = Arc::new(FailMetadataWrite {
        inner: Arc::clone(&inner),
        faults,
    });
    let refreshed = CappedCcrStore {
        store: BlobCcrStore::new(Arc::clone(&failing), None).unwrap(),
        blobs: failing,
        ttl: None,
        max_bytes: None,
    };

    assert!(refreshed.put(content).retained());
    assert_eq!(
        read_semantic_time(Arc::clone(&inner), &token).unwrap(),
        None
    );
    assert_eq!(
        read_import_intent(Arc::clone(&inner), &token).unwrap(),
        None
    );
    assert_eq!(
        prune(&inner, Some(Duration::from_secs(3_600)), None, None).unwrap(),
        (0, 0, 1, content.len() as u64),
        "the physical write time must replace stale imported age when timestamp refresh fails"
    );
    assert_eq!(original.get(&token).as_deref(), Some(content));
}

#[test]
fn import_refuses_to_replace_different_content_at_a_legacy_token() {
    let dir = tempfile::tempdir().unwrap();
    let content = "source legacy content";
    let token = tinyjuice::cache::short_hash(content);
    let source_path = dir.path().join(&token);
    std::fs::write(&source_path, content).unwrap();
    let blobs = open_file_blobs(dir.path()).unwrap();
    let key = format!("{CCR_PREFIX}/{token}");
    run_blob({
        let blobs = Arc::clone(&blobs);
        let key = key.clone();
        async move {
            blobs
                .put(&key, b"different target content".to_vec(), None)
                .await
        }
    })
    .unwrap();

    let error = import_legacy_files(dir.path(), &blobs, None).unwrap_err();
    assert!(error.contains("conflicts with different stored content"));
    assert!(source_path.exists(), "the unimported source is retained");
    let existing = run_blob({
        let blobs = Arc::clone(&blobs);
        async move { blobs.get(&key).await }
    })
    .unwrap()
    .unwrap();
    assert_eq!(existing.bytes, b"different target content");
}

#[test]
fn retry_after_timestamp_write_failure_preserves_the_source_file_age() {
    let dir = tempfile::tempdir().unwrap();
    let content = "legacy ccr whose age must survive an interrupted import";
    let token = tinyjuice::cache::short_hash(content);
    let source_path = dir.path().join(&token);
    std::fs::write(&source_path, content).unwrap();
    let source_time = SystemTime::now() - Duration::from_secs(20);
    std::fs::File::open(&source_path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(source_time))
        .unwrap();

    let inner = open_file_blobs(dir.path()).unwrap();
    let faults = FaultPlan::new();
    faults.fail_operation_nth("metadata_put", 1, ErrorKind::Backend);
    let failing: Arc<dyn BlobStore> = Arc::new(FailMetadataWrite {
        inner: Arc::clone(&inner),
        faults: faults.clone(),
    });

    assert!(import_legacy_files(dir.path(), &failing, None).is_err());
    assert!(source_path.exists(), "failed import must retain its source");
    let body_key = format!("{CCR_PREFIX}/{token}");
    assert!(
        run_blob({
            let inner = Arc::clone(&inner);
            let body_key = body_key.clone();
            async move { inner.get(&body_key).await }
        })
        .unwrap()
        .is_some(),
        "the fault occurs after the body has been stored"
    );

    faults.clear();
    import_legacy_files(dir.path(), &failing, None).unwrap();
    assert!(
        !source_path.exists(),
        "retry completes and retires the source"
    );
    assert_eq!(
        read_semantic_time(Arc::clone(&inner), &token).unwrap(),
        Some(unix_epoch_ms(source_time))
    );
    assert_eq!(
        read_import_intent(Arc::clone(&inner), &token).unwrap(),
        None
    );
}

#[test]
fn imports_released_v01010_disk_tier_file_and_preserves_its_age() {
    const TOKEN: &str = "1e66b4cff1835002102d821ffee64f75";
    const CONTENT: &str =
        include_str!("../tests/fixtures/legacy-ccr/v0.10.10/1e66b4cff1835002102d821ffee64f75");

    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join(TOKEN);
    std::fs::write(&source_path, CONTENT).unwrap();
    let source_time = SystemTime::now() - Duration::from_secs(5);
    std::fs::File::open(&source_path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(source_time))
        .unwrap();

    assert_eq!(tinyjuice::cache::short_hash(CONTENT), TOKEN);
    let blobs = open_file_blobs(dir.path()).unwrap();
    import_legacy_files(dir.path(), &blobs, Some(Duration::from_secs(3600))).unwrap();

    assert!(!source_path.exists(), "the verified legacy file is retired");
    let store = BlobCcrStore::new(Arc::clone(&blobs), None).unwrap();
    assert_eq!(store.get(TOKEN).as_deref(), Some(CONTENT));
    assert_eq!(
        read_semantic_time(Arc::clone(&blobs), TOKEN).unwrap(),
        Some(unix_epoch_ms(source_time))
    );
}
