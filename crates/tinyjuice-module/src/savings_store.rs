//! The savings totals on the host's document port.
//!
//! One document, `juice.savings/totals`, holds the whole aggregate in the
//! module's scope. The in-memory ledger stays the working copy: it is loaded
//! from the document on first use and the document is rewritten after every
//! change. A host that serves no storage leaves the totals in memory only.

use std::sync::Arc;

use tinybus::{Connection, Result as BusResult};
use tinyjuice::savings::ledger::SavingsLedger;
use tinyjuice_bus::savings::{
    RecordSavingsRequest, SAVINGS_DOC_ID, SavingsAggregate, StoredSavings,
};
use tinyjuice_bus::storage::SAVINGS_COLLECTION;
use tinystoragedrivers_core::{DocumentStore, DocumentStoreExt, Precondition};

/// Working state behind one lock, so load, change and write never interleave.
#[derive(Default)]
struct State {
    docs: Option<Arc<dyn DocumentStore>>,
    /// The ledger has been reconciled with the stored document (or the module
    /// has no storage connection at all and stays in memory).
    loaded: bool,
    /// The ledger changed before the stored document could be read.
    dirty: bool,
    /// A reset happened before the stored document could be read, so the
    /// stored totals must be discarded rather than added to.
    reset_early: bool,
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
        state.dirty |= !state.loaded;
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
        if !state.loaded {
            state.dirty = true;
            state.reset_early = true;
        }
        self.persist(&state).await;
    }

    /// Adopt `legacy` when nothing has been recorded. Answers whether it did;
    /// fails while the stored totals cannot be read, so the host retries.
    pub(crate) async fn import(
        &self,
        connection: Option<&Connection>,
        legacy: SavingsAggregate,
    ) -> BusResult<bool> {
        let mut state = self.state.lock().await;
        self.load(&mut state, connection).await;
        if !state.loaded {
            return Err(tinybus::Error::failed("savings storage is not readable yet"));
        }
        if self.ledger.stats().total.events != 0 {
            log::debug!("[tinyjuice_module][savings] import skipped: totals exist");
            return Ok(false);
        }
        self.ledger.replace(legacy);
        self.persist(&state).await;
        Ok(true)
    }

    /// Reach the document port and reconcile the ledger with its document.
    ///
    /// Changes made before the document could be read are kept: they are
    /// added to the stored totals (or replace them after an early reset) and
    /// the result is written back.
    async fn load(&self, state: &mut State, connection: Option<&Connection>) {
        if state.loaded {
            return;
        }
        let Some(connection) = connection else {
            state.loaded = true;
            return;
        };
        if state.docs.is_none() {
            state.docs = crate::storage::documents(connection.clone()).await;
        }
        let Some(docs) = state.docs.as_ref() else {
            return;
        };
        let stored = match docs
            .get_as::<StoredSavings>(SAVINGS_COLLECTION, SAVINGS_DOC_ID)
            .await
        {
            Ok(found) => found.map(|found| found.doc.into_aggregate()),
            Err(error) => {
                log::warn!("[tinyjuice_module][savings] load failed, retrying later: {error}");
                return;
            }
        };
        if let Some(stored) = stored.filter(|_| !state.reset_early) {
            let early = self.ledger.stats();
            self.ledger.replace(stored);
            self.ledger.absorb(&early);
        }
        state.loaded = true;
        if state.dirty {
            log::debug!("[tinyjuice_module][savings] reconciled early changes with storage");
            state.dirty = false;
            state.reset_early = false;
            self.persist(state).await;
        }
    }

    async fn persist(&self, state: &State) {
        // Nothing is written until the stored document has been read, so an
        // unread older document is never overwritten blindly.
        if !state.loaded {
            return;
        }
        let Some(docs) = state.docs.as_ref() else {
            return;
        };
        let body = StoredSavings::from_aggregate(&self.ledger.stats(), now_ms());
        if let Err(error) = docs
            .put_as(SAVINGS_COLLECTION, SAVINGS_DOC_ID, &body, Precondition::None)
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
