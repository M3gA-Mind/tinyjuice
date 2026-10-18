//! The savings totals on the host's document port.
//!
//! One document, `juice.savings/totals`, holds the whole aggregate in the
//! module's scope. The in-memory ledger stays the working copy: it is loaded
//! from the document on first use and the document is rewritten after every
//! change. A host that serves no storage leaves the totals in memory only.

use std::sync::Arc;

use tinybus::Connection;
use tinyjuice::savings::ledger::SavingsLedger;
use tinyjuice_bus::savings::{
    RecordSavingsRequest, SAVINGS_DOC_ID, SavingsAggregate, StoredSavings,
};
use tinyjuice_bus::storage::SAVINGS_COLLECTION;
use tinystoragedrivers_core::{DocumentStore, DocumentStoreExt, Precondition};

/// Working state behind one lock, so load, change and write never interleave.
#[derive(Default)]
struct State {
    /// `Some` once the document port was reached (or known absent).
    resolved: bool,
    docs: Option<Arc<dyn DocumentStore>>,
    /// The ledger has been seeded from the stored document.
    loaded: bool,
}

/// The module's savings ledger and its storage.
pub(crate) struct Savings {
    ledger: SavingsLedger,
    state: tokio::sync::Mutex<State>,
}

impl Savings {
    pub(crate) fn new() -> Self {
        Self {
            ledger: SavingsLedger::new(),
            state: tokio::sync::Mutex::new(State::default()),
        }
    }

    pub(crate) async fn record(&self, connection: Option<&Connection>, req: &RecordSavingsRequest) {
        let mut state = self.state.lock().await;
        self.load(&mut state, connection).await;
        self.ledger.record(req);
        self.persist(&state).await;
    }

    pub(crate) async fn stats(&self, connection: Option<&Connection>) -> SavingsAggregate {
        let mut state = self.state.lock().await;
        self.load(&mut state, connection).await;
        self.ledger.stats()
    }

    pub(crate) async fn reset(&self, connection: Option<&Connection>) {
        let mut state = self.state.lock().await;
        self.load(&mut state, connection).await;
        self.ledger.reset();
        self.persist(&state).await;
    }

    /// Adopt `legacy` when nothing has been recorded. Answers whether it did.
    pub(crate) async fn import(
        &self,
        connection: Option<&Connection>,
        legacy: SavingsAggregate,
    ) -> bool {
        let mut state = self.state.lock().await;
        self.load(&mut state, connection).await;
        if self.ledger.stats().total.events != 0 {
            log::debug!("[tinyjuice_module][savings] import skipped: totals exist");
            return false;
        }
        self.ledger.replace(legacy);
        self.persist(&state).await;
        true
    }

    /// Reach the document port once and seed the ledger from its document.
    async fn load(&self, state: &mut State, connection: Option<&Connection>) {
        if !state.resolved {
            if let Some(connection) = connection {
                state.docs = crate::storage::documents(connection.clone()).await;
            }
            state.resolved = state.docs.is_some() || connection.is_none();
        }
        if state.loaded {
            return;
        }
        let Some(docs) = state.docs.as_ref() else {
            return;
        };
        match docs
            .get_as::<StoredSavings>(SAVINGS_COLLECTION, SAVINGS_DOC_ID)
            .await
        {
            Ok(found) => {
                if let Some(found) = found {
                    self.ledger.replace(found.doc.into_aggregate());
                }
                state.loaded = true;
            }
            Err(error) => {
                log::warn!("[tinyjuice_module][savings] load failed, retrying later: {error}");
            }
        }
    }

    async fn persist(&self, state: &State) {
        let Some(docs) = state.docs.as_ref() else {
            return;
        };
        let body = StoredSavings::from_aggregate(&self.ledger.stats(), now_ms());
        if let Err(error) = docs
            .put_as(
                SAVINGS_COLLECTION,
                SAVINGS_DOC_ID,
                &body,
                Precondition::None,
            )
            .await
        {
            log::debug!("[tinyjuice_module][savings] write failed: {error}");
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
#[path = "savings_store_tests.rs"]
pub(crate) mod tests;
