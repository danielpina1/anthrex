//! Milestone 9.8 (decision 37): the client's copy of the discovered model catalogs.
//! Pure state; M9.8.10 fills this file out with the picker.

use proto::ModelCatalog;

/// The catalogs `DaemonMsg::Models` has delivered, one per runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalogs {
    pub list: Vec<ModelCatalog>,
}

impl Catalogs {
    /// Stores `incoming`, a catalog replacing the one already held for its runtime (a
    /// `ListModels` for one runtime says nothing about the other).
    pub fn absorb(&mut self, incoming: Vec<ModelCatalog>) {
        for catalog in incoming {
            match self.list.iter_mut().find(|c| c.runtime == catalog.runtime) {
                Some(held) => *held = catalog,
                None => self.list.push(catalog),
            }
        }
    }
}
