//! TinyJuice's storage vocabulary for hosts composing its owner ports.

use tinystoragedrivers_types::CollectionSpec;

/// CCR originals are stored below `juice/ccr/<token>`.
pub const CCR_PREFIX: &str = "juice/ccr";

/// Legacy prefix used before the owner moved originals into `juice/ccr`.
pub const LEGACY_CCR_PREFIX: &str = "tinyjuice_ccr/";

/// The owner-scoped savings totals document collection.
pub const SAVINGS_COLLECTION: &str = "juice.savings";

/// Collections a host should provision when it composes TinyJuice storage.
///
/// Blob prefixes are provisioned by the blob engine itself; only document
/// collections need a declaration in this vocabulary.
#[must_use]
pub fn collections() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new(SAVINGS_COLLECTION)]
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
