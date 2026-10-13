use super::*;

#[test]
fn round_trips() {
    let original = "ccr round-trip unique payload alpha ".repeat(50);
    let hash = offload(&original);
    assert_eq!(hash.len(), HASH_BYTES * 2);
    assert_eq!(retrieve(&hash).as_deref(), Some(original.as_str()));
}

#[test]
fn idempotent_hash() {
    let a = offload("ccr idempotent unique payload bravo content");
    let b = offload("ccr idempotent unique payload bravo content");
    assert_eq!(a, b);
}

#[test]
fn missing_hash_is_none() {
    assert!(retrieve("ffffffffffffffffffffffffffffffff").is_none());
}

#[test]
fn byte_cap_evicts_oldest() {
    let mut inner = Inner::default();
    // 10 entries of 100 bytes; cap at 500 bytes ⇒ keep ~5 newest.
    for i in 0..10 {
        inner.insert(format!("h{i}"), "x".repeat(100), 1000, 500);
    }
    assert!(
        inner.total_bytes <= 500,
        "byte cap held: {}",
        inner.total_bytes
    );
    assert!(!inner.map.contains_key("h0"), "oldest evicted");
    assert!(inner.map.contains_key("h9"), "newest retained");
}

#[test]
fn oversized_single_entry_is_not_retained() {
    // One entry larger than the byte cap can't be kept; insert reports
    // non-retention so the router won't advertise it as recoverable.
    let mut inner = Inner::default();
    let retained = inner.insert("big".into(), "x".repeat(200), 100, 100);
    assert!(!retained, "oversized single entry must report not-retained");
    assert!(!inner.map.contains_key("big"));
    assert_eq!(inner.total_bytes, 0);
}

#[test]
fn rejects_path_traversal_tokens() {
    // Non-hex / wrong-length tokens are rejected before any disk join.
    assert!(!is_valid_token("../../state/config.toml"));
    assert!(!is_valid_token("..%2f..%2fetc"));
    assert!(!is_valid_token("deadbeef")); // too short
    assert!(!is_valid_token(&"g".repeat(32))); // non-hex
    assert!(is_valid_token(&"a1b2c3d4".repeat(4))); // 32 hex chars
    // retrieve() returns None for an invalid token regardless of cache state.
    assert!(retrieve("../../state/config.toml").is_none());
}

#[test]
fn within_cap_entry_is_retained() {
    let mut inner = Inner::default();
    assert!(inner.insert("ok".into(), "x".repeat(50), 100, 100));
    assert!(inner.map.contains_key("ok"));
}

#[test]
fn entry_cap_evicts_oldest() {
    let mut inner = Inner::default();
    for i in 0..60 {
        inner.insert(format!("e{i}"), format!("content-{i}"), 50, usize::MAX);
    }
    assert!(inner.map.len() <= 50);
    assert!(!inner.map.contains_key("e0"));
}

#[test]
fn reoffloading_existing_entry_refreshes_ttl_and_order() {
    let mut inner = Inner::default();
    assert!(inner.insert("old".into(), "old payload".into(), 2, usize::MAX));
    assert!(inner.insert("fresh".into(), "fresh payload".into(), 2, usize::MAX));
    let stale_created = Instant::now() - Duration::from_secs(60);
    inner.map.get_mut("old").unwrap().created = stale_created;

    assert!(inner.insert("old".into(), "old payload".into(), 2, usize::MAX));
    assert!(
        inner.map["old"].created > stale_created,
        "existing entry timestamp should refresh"
    );
    assert_eq!(inner.order.back().map(String::as_str), Some("old"));

    assert!(inner.insert("third".into(), "third payload".into(), 2, usize::MAX));
    assert!(inner.map.contains_key("old"));
    assert!(!inner.map.contains_key("fresh"));
}

#[test]
fn range_retrieval_lines_and_bytes() {
    let original = "line0\nline1\nline2\nline3\nline4";
    let hash = offload(original);
    assert_eq!(
        retrieve_range(&hash, 1, 3, RangeUnit::Lines).as_deref(),
        Some("line1\nline2")
    );
    assert_eq!(
        retrieve_range(&hash, 0, 5, RangeUnit::Bytes).as_deref(),
        Some("line0")
    );
    // Out-of-bounds end clamps.
    assert_eq!(
        retrieve_range(&hash, 4, 999, RangeUnit::Lines).as_deref(),
        Some("line4")
    );
}

#[test]
fn memory_store_is_isolated_from_global_cache() {
    let store = MemoryCcrStore::new(10, 10_000);
    let original = "isolated memory store payload delta ".repeat(20);
    let put = store.put(&original);

    assert!(put.retained());
    assert_eq!(store.get(put.token()).as_deref(), Some(original.as_str()));
    assert_eq!(retrieve(put.token()), None, "global cache must not see it");
}

#[test]
fn memory_store_rejects_oversized_originals() {
    let store = MemoryCcrStore::new(10, 16);
    let original = "oversized isolated payload echo ".repeat(20);
    let put = store.put(&original);

    assert!(!put.retained());
    assert_eq!(store.get(put.token()), None);
    assert_eq!(store.stats(), (0, 0));
}

#[test]
fn memory_store_rejects_malformed_tokens_before_disk_lookup() {
    let dir = std::env::temp_dir().join(format!(
        "tj-memory-ccr-{}",
        short_hash("memory-store-malformed-token")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let store = MemoryCcrStore::new(10, 10_000).with_disk_root(dir.clone());

    assert_eq!(store.get("../../state/config.toml"), None);
    assert_eq!(
        store.get_range("../../state/config.toml", 0, 1, RangeUnit::Lines),
        None
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn disk_tier_survives_memory_miss() {
    let dir = std::env::temp_dir().join(format!("tj-ccr-{}", short_hash("disk-test-seed")));
    let _ = std::fs::remove_dir_all(&dir);
    enable_disk_tier(dir.clone());
    let original = "disk tier unique payload charlie ".repeat(40);
    let hash = offload(&original);
    // Simulate memory eviction by clearing the in-memory map directly.
    {
        let mut inner = global().lock().unwrap_or_else(|p| p.into_inner());
        inner.map.remove(&hash);
    }
    assert_eq!(
        retrieve(&hash).as_deref(),
        Some(original.as_str()),
        "disk fallback"
    );
    // Disable the tier for other tests and clean up.
    *disk_root().write().unwrap() = None;
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn offload_with_hash_round_trips() {
    let original = "ccr precomputed-hash unique payload delta ".repeat(30);
    let hash = short_hash(&original);
    assert!(offload_checked_with_hash(&hash, &original));
    assert_eq!(retrieve(&hash).as_deref(), Some(original.as_str()));
    // Matches the hashing path exactly.
    assert_eq!(offload_checked(&original).0, hash);
}

/// Write a token-named file with a back-dated mtime.
fn write_aged(dir: &Path, name: &str, bytes: usize, age: Duration) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "x".repeat(bytes)).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
    path
}

fn token(fill: char) -> String {
    fill.to_string().repeat(32)
}

#[test]
fn gc_removes_expired_keeps_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let old = write_aged(dir.path(), &token('a'), 10, Duration::from_secs(600));
    let fresh = write_aged(dir.path(), &token('b'), 20, Duration::ZERO);

    let stats = gc_disk_dir(dir.path(), Some(Duration::from_secs(60)), None).unwrap();
    assert_eq!(stats.removed, 1);
    assert_eq!(stats.freed_bytes, 10);
    assert_eq!(stats.kept, 1);
    assert_eq!(stats.kept_bytes, 20);
    assert!(!old.exists());
    assert!(fresh.exists());
}

#[test]
fn gc_evicts_oldest_over_budget() {
    let dir = tempfile::tempdir().unwrap();
    let oldest = write_aged(dir.path(), &token('a'), 100, Duration::from_secs(30));
    let middle = write_aged(dir.path(), &token('b'), 100, Duration::from_secs(20));
    let newest = write_aged(dir.path(), &token('c'), 100, Duration::from_secs(10));

    let stats = gc_disk_dir(dir.path(), None, Some(250)).unwrap();
    assert_eq!(stats.removed, 1, "one eviction brings 300 under 250");
    assert_eq!(stats.freed_bytes, 100);
    assert_eq!(stats.kept, 2);
    assert!(!oldest.exists(), "oldest-by-mtime goes first");
    assert!(middle.exists());
    assert!(newest.exists());
}

#[test]
fn gc_ignores_non_token_files_and_missing_dir() {
    let dir = tempfile::tempdir().unwrap();
    let stray = dir.path().join("README.txt");
    std::fs::write(&stray, "not a ccr entry").unwrap();

    let stats = gc_disk_dir(dir.path(), Some(Duration::ZERO), Some(0)).unwrap();
    assert_eq!(stats, GcStats::default());
    assert!(stray.exists(), "non-token files are never touched");

    let missing = dir.path().join("does-not-exist");
    assert_eq!(
        gc_disk_dir(&missing, None, None).unwrap(),
        GcStats::default()
    );
}

#[test]
fn gc_protects_named_entry_from_budget_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let protected_name = token('a');
    let protected = write_aged(dir.path(), &protected_name, 100, Duration::from_secs(30));
    let newer = write_aged(dir.path(), &token('b'), 100, Duration::from_secs(10));

    let stats = gc_dir_protecting(dir.path(), None, Some(150), Some(&protected_name)).unwrap();
    assert!(
        protected.exists(),
        "protected entry survives even when oldest"
    );
    assert!(!newer.exists(), "eviction falls through to the next-oldest");
    assert_eq!(stats.removed, 1);
    assert_eq!(stats.kept, 1);
}

#[test]
fn disk_entry_ttl_uses_file_mtime() {
    let dir = std::env::temp_dir().join(format!("tj-ccr-ttl-{}", short_hash("disk-ttl")));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    std::fs::write(&path, "disk ttl payload").unwrap();

    assert!(!disk_entry_expired(&path, None));
    assert!(!disk_entry_expired(&path, Some(Duration::from_secs(60))));
    assert!(disk_entry_expired(&path, Some(Duration::ZERO)));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn isolated_store_disk_survives_memory_eviction_and_validates_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryCcrStore::new(1, 8).with_disk_root(dir.path().to_owned());
    let result = store.put("larger than memory capacity");
    assert!(result.retained());
    assert_eq!(store.stats(), (0, 0));
    assert_eq!(
        store.get(result.token()).as_deref(),
        Some("larger than memory capacity")
    );
    assert_eq!(
        store
            .get_range(result.token(), 0, 6, RangeUnit::Bytes)
            .as_deref(),
        Some("larger")
    );
    assert_eq!(store.get("../escape"), None);
    assert_eq!(
        store.get_range(&"0".repeat(32), 0, 1, RangeUnit::Lines),
        None
    );
}

#[test]
fn isolated_store_ttl_expires_memory_and_disk_and_updates_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = MemoryCcrStore::default()
        .with_ttl(Some(Duration::ZERO))
        .with_disk_root(dir.path().to_owned());
    let result = store.put("expired payload");
    assert!(result.retained());
    assert_eq!(store.stats(), (1, 15));
    assert_eq!(store.get(result.token()), None);
    assert_eq!(store.stats(), (0, 0));
    assert!(!dir.path().join(result.token()).exists());
}

#[test]
fn isolated_store_disk_setup_and_write_failures_do_not_claim_retention() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("file");
    std::fs::write(&blocked, "block").unwrap();
    let store = MemoryCcrStore::new(1, 1).with_disk_root(blocked);
    assert!(!store.put("too large").retained());
    let root = dir.path().join("disk");
    let store = MemoryCcrStore::new(1, 1).with_disk_root(root.clone());
    std::fs::remove_dir(&root).unwrap();
    assert!(!store.put("too large").retained());
}

#[test]
fn garbage_collection_skips_nonentries_and_protects_inflight_original() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes"), "unrelated").unwrap();
    std::fs::create_dir(dir.path().join("a".repeat(32))).unwrap();
    let protected = short_hash("protected");
    std::fs::write(dir.path().join(&protected), "protected").unwrap();
    let removed = short_hash("removed");
    std::fs::write(dir.path().join(&removed), "removed").unwrap();
    let stats = gc_dir_protecting(dir.path(), None, Some(0), Some(&protected)).unwrap();
    assert_eq!(stats.removed, 1);
    assert_eq!(stats.kept, 1);
    assert_eq!(stats.kept_bytes, 9);
    assert!(dir.path().join(&protected).exists());
    assert!(dir.path().join("notes").exists());
    assert_eq!(
        gc_disk_dir(&dir.path().join("absent"), None, None).unwrap(),
        GcStats::default()
    );
    assert!(gc_disk_dir(&dir.path().join("notes"), None, None).is_err());
}

#[test]
fn default_range_retrieval_clamps_utf8_and_empty_ranges() {
    let store = MemoryCcrStore::default();
    let put = store.put("é\nsecond");
    assert_eq!(
        store
            .get_range(put.token(), 1, 2, RangeUnit::Bytes)
            .as_deref(),
        Some("é")
    );
    assert_eq!(
        store
            .get_range(put.token(), 1, 99, RangeUnit::Lines)
            .as_deref(),
        Some("second")
    );
    assert_eq!(
        store
            .get_range(put.token(), 8, 9, RangeUnit::Lines)
            .as_deref(),
        Some("")
    );
    assert_eq!(
        store
            .get_range(put.token(), 2, 1, RangeUnit::Bytes)
            .as_deref(),
        Some("")
    );
}

#[test]
fn global_adapter_supports_ranges_and_unconfigured_disk_gc() {
    let store = GlobalCcrStore;
    let put = store.put("global ranged retrieval fixture\nsecond");
    assert!(put.retained());
    assert_eq!(
        store
            .get_range(put.token(), 1, 2, RangeUnit::Lines)
            .as_deref(),
        Some("second")
    );
    assert!(stats().0 > 0);
    assert!(gc_disk_tier(None, None).is_ok());
    assert!(!disk_entry_expired(
        &PathBuf::from("/nonexistent-tinyjuice-fixture"),
        Some(Duration::ZERO)
    ));
}

#[test]
fn isolated_store_recovers_its_contents_after_lock_poisoning() {
    let store = MemoryCcrStore::default();
    let put = store.put("poison recovery fixture");
    let _ = std::panic::catch_unwind(|| {
        let _lock = store.inner.lock().unwrap();
        panic!("intentional isolated-store lock poison");
    });
    assert_eq!(
        store.get(put.token()).as_deref(),
        Some("poison recovery fixture")
    );
    assert_eq!(store.stats(), (1, 23));
    let next = store.put("second fixture");
    assert!(next.retained());
}

/// The durable tier is process-global; tests that install one take turns.
static DURABLE_TIER: Mutex<()> = Mutex::new(());

#[test]
fn durable_tier_mirrors_offloads_and_serves_memory_misses() {
    let _turn = DURABLE_TIER.lock().unwrap_or_else(|p| p.into_inner());
    let durable = Arc::new(MemoryCcrStore::default());
    set_durable_tier(Some(durable.clone()));

    let mirrored = "durable tier mirrored offload fixture";
    let (hash, retained) = offload_checked(mirrored);
    assert!(retained);
    assert_eq!(durable.get(&hash).as_deref(), Some(mirrored));

    // An original only the durable tier holds (evicted or from before a
    // restart) is still retrievable, whole and by range.
    let only_durable = durable.put("durable only\nsecond line");
    assert_eq!(
        retrieve(only_durable.token()).as_deref(),
        Some("durable only\nsecond line")
    );
    assert_eq!(
        retrieve_range(only_durable.token(), 1, 2, RangeUnit::Lines).as_deref(),
        Some("second line")
    );

    set_durable_tier(None);
    assert_eq!(retrieve(only_durable.token()), None, "cleared tier is gone");
}

/// Retains every original, but under a token of its own rather than the
/// content hash callers advertise.
struct OtherToken;

impl CcrStore for OtherToken {
    fn put(&self, _content: &str) -> CcrPutResult {
        CcrPutResult::new("0123456789abcdef01234567".to_string(), true)
    }
    fn get(&self, _token: &str) -> Option<String> {
        None
    }
    fn get_range(&self, _: &str, _: usize, _: usize, _: RangeUnit) -> Option<String> {
        None
    }
}

#[test]
fn durable_retention_requires_the_recovery_token() {
    let _turn = DURABLE_TIER.lock().unwrap_or_else(|p| p.into_inner());
    set_durable_tier(Some(Arc::new(OtherToken)));
    // Larger than the memory cap, so only the durable tier could retain it.
    let oversized = "x".repeat(DEFAULT_MAX_BYTES + 1);
    let (_hash, retained) = offload_checked(&oversized);
    set_durable_tier(None);
    assert!(!retained, "a store keyed by another token cannot serve the hash");
}
