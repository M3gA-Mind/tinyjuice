//! Persisted savings totals and requests shared with the compression module.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-key (model / compressor) rolled-up savings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingsBucket {
    /// Number of shrinking compactions; saturates at `u64::MAX`.
    pub events: u64,
    /// Sum of estimated tokens before compaction.
    pub original_tokens: u64,
    /// Sum of estimated tokens after compaction.
    pub compacted_tokens: u64,
    /// Sum of positive pre/post token differences.
    pub tokens_saved: u64,
    /// Estimated dropped-input cost from host prices; saturates at `f64::MAX`.
    pub cost_saved_usd: f64,
}

/// The full savings snapshot returned to callers / the dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingsAggregate {
    /// Overall totals across every compaction.
    pub total: SavingsBucket,
    /// Breakdown by the model the savings were attributed to.
    pub by_model: HashMap<String, SavingsBucket>,
    /// Breakdown by which compressor produced the saving.
    pub by_compressor: HashMap<String, SavingsBucket>,
}

/// One compaction attributed to the model that would consume its output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordSavingsRequest {
    /// Model identifier used for the dashboard breakdown and host price lookup.
    pub model: String,
    /// Compressor identifier used for the dashboard breakdown.
    pub compressor: String,
    /// Estimated input tokens before compaction.
    pub original_tokens: u64,
    /// Estimated input tokens after compaction.
    pub compacted_tokens: u64,
    /// Host-resolved price per million input tokens; absent means unpriced.
    pub input_per_mtok_usd: Option<f64>,
}
