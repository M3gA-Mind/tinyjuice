//! Tests for the CCR durable tier on host storage: blobs on the host port,
//! expiry, the in-memory fallback, and single-worker safety.

use super::*;
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinyjuice::cache::store::short_hash;
use tinystoragedrivers_busproxy::{PerCallerEndpoint, ServedStorage, serve};
use tinystoragedrivers_client::StorageServer;
use tinystoragedrivers_core::{MemoryStorage, Scope, ScopedStorage};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Rig {
    module: Connection,
    backend: MemoryStorage,
    _served: Option<ServedStorage>,
}

async fn rig(serve_storage: bool) -> Result<Rig, Box<dyn std::error::Error>> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let backend = MemoryStorage::new();
    let host = Connection::connect(bus.connect().await?).await?;
    let served = if serve_storage {
        let bound = backend.clone();
        let endpoint = PerCallerEndpoint::new(move |_sender| {
            let scoped: ScopedStorage = bound.for_scope(&Scope::local()).ok()?;
            Some(StorageServer::new(scoped))
        });
        Some(serve(&host, Arc::new(endpoint)).await?)
    } else {
        None
    };
    let module = Connection::connect(bus.connect().await?).await?;
    // Keep the host connection alive for the life of the broker.
    std::mem::forget(host);
    Ok(Rig {
        module,
        backend,
        _served: served,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn originals_land_as_blobs_on_the_host_port() -> TestResult {
    let rig = rig(true).await?;
    let store = durable_tier(rig.module.clone(), Some(Duration::from_secs(600)))
        .await
        .ok_or("host serves storage")?;
    let content = "ccr original on the host blob port\nsecond line";
    let put = store.put(content);
    assert!(put.retained());

    let host_blobs = Arc::clone(rig.backend.for_scope(&Scope::local())?.blobs());
    let key = format!("{}/{}", tinyjuice_drivers::CCR_PREFIX, short_hash(content));
    let blob = host_blobs.get(&key).await?.ok_or("blob is on the host")?;
    assert_eq!(blob.bytes, content.as_bytes());

    assert_eq!(store.get(put.token()).as_deref(), Some(content));
    assert_eq!(
        store.get_range(put.token(), 1, 2, RangeUnit::Lines).as_deref(),
        Some("second line")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_storage_host_there_is_no_durable_tier() -> TestResult {
    let rig = rig(false).await?;
    assert!(durable_tier(rig.module.clone(), None).await.is_none());
    Ok(())
}

/// The module has two workers at most; a store call made directly from async
/// code must not wait on a bus reply only that worker could deliver.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn store_calls_on_the_only_worker_do_not_deadlock() -> TestResult {
    let rig = rig(true).await?;
    let store = durable_tier(rig.module.clone(), None)
        .await
        .ok_or("host serves storage")?;
    tokio::time::timeout(Duration::from_secs(20), async {
        let put = store.put("single worker original");
        assert!(put.retained());
        assert_eq!(
            store.get(put.token()).as_deref(),
            Some("single worker original")
        );
    })
    .await?;
    Ok(())
}
