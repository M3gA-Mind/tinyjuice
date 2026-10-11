//! Model-attributed savings aggregation and best-effort snapshot persistence.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use tinyjuice_bus::savings::{RecordSavingsRequest, SavingsAggregate, SavingsBucket};

#[derive(Default)]
struct State {
    aggregate: SavingsAggregate,
    snapshot_path: Option<PathBuf>,
}

/// A thread-safe ledger scoped to its caller, without a host pricing catalog.
///
/// Counts are estimates supplied by the compressor. Monetary savings use only
/// the host-provided input rate, with unpriced models reporting zero dollars.
#[derive(Default)]
pub struct SavingsLedger {
    state: Mutex<State>,
}

impl SavingsLedger {
    /// Creates an empty in-memory ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects a snapshot file and loads it once per path.
    ///
    /// Repeated configuration of the same path does not reload stale disk
    /// state. Missing or malformed snapshots leave the current totals intact,
    /// matching the original host ledger. I/O failures are best-effort.
    pub fn configure(&self, path: &Path) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.snapshot_path.as_deref() == Some(path) {
            return;
        }
        let loaded = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        state.snapshot_path = Some(path.to_path_buf());
        if let Some(aggregate) = loaded {
            state.aggregate = aggregate;
        }
    }

    /// Records a strictly smaller compaction and snapshots its totals.
    ///
    /// Equal or expanded output is a no-op. Negative or non-finite input
    /// prices are treated as unpriced so persisted totals remain valid JSON.
    pub fn record(&self, request: &RecordSavingsRequest) {
        if request.original_tokens <= request.compacted_tokens {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let cost = request
            .input_per_mtok_usd
            .filter(|price| price.is_finite() && *price >= 0.0)
            .map_or(0.0, |price| {
                (request.original_tokens - request.compacted_tokens) as f64 / 1_000_000.0 * price
            });
        let aggregate = &mut state.aggregate;
        add(&mut aggregate.total, request, cost);
        add(
            aggregate.by_model.entry(request.model.clone()).or_default(),
            request,
            cost,
        );
        add(
            aggregate
                .by_compressor
                .entry(request.compressor.clone())
                .or_default(),
            request,
            cost,
        );
        persist(&state);
    }

    /// Returns a consistent copy of all current totals.
    #[must_use]
    pub fn stats(&self) -> SavingsAggregate {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .aggregate
            .clone()
    }

    /// Clears totals and writes the empty persisted snapshot.
    pub fn reset(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.aggregate = SavingsAggregate::default();
        persist(&state);
    }
}

fn add(bucket: &mut SavingsBucket, request: &RecordSavingsRequest, cost: f64) {
    bucket.events += 1;
    bucket.original_tokens += request.original_tokens;
    bucket.compacted_tokens += request.compacted_tokens;
    bucket.tokens_saved += request.original_tokens - request.compacted_tokens;
    bucket.cost_saved_usd += cost;
}

fn persist(state: &State) {
    let Some(path) = state.snapshot_path.as_ref() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string(&state.aggregate) {
        Ok(json) => {
            if let Err(error) = std::fs::write(path, json) {
                log::debug!("[tokenjuice][savings] snapshot write failed: {error}");
            }
        }
        Err(error) => log::debug!("[tokenjuice][savings] snapshot serialize failed: {error}"),
    }
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
