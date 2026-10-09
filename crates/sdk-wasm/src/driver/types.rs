//! What the driver hands the page by value: what a restore found and where
//! `run` is.

use anyswap_sdk::driver::{RestoreReport as SdkRestoreReport, RunState as SdkRunState};
use wasm_bindgen::prelude::*;

use super::error::DriverError;

/// What `restoreAll` found among the swaps the server lists and the store
/// lacks.
#[wasm_bindgen]
pub struct RestoreReport {
    inner: SdkRestoreReport,
}

impl From<SdkRestoreReport> for RestoreReport {
    fn from(inner: SdkRestoreReport) -> Self {
        Self { inner }
    }
}

#[wasm_bindgen]
impl RestoreReport {
    /// The ids of the swaps now in the store.
    #[wasm_bindgen(getter)]
    pub fn restored(&self) -> Vec<String> {
        self.inner
            .restored
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Each swap that failed a check, with the check; unsigned for one the
    /// server holds no signed request for.
    #[wasm_bindgen(getter)]
    pub fn failed(&self) -> Vec<RestoreFailure> {
        self.inner
            .failed
            .iter()
            .map(|(swap_id, error)| RestoreFailure {
                swap_id: swap_id.to_string(),
                error: DriverError::from(error),
            })
            .collect()
    }
}

/// One swap `restoreAll` could not restore, with why.
#[wasm_bindgen(getter_with_clone)]
pub struct RestoreFailure {
    #[wasm_bindgen(readonly, js_name = swapId)]
    pub swap_id: String,
    #[wasm_bindgen(readonly)]
    pub error: DriverError,
}

/// Where `run` is. `Stopped` is final.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle = "Idle",
    Running = "Running",
    Stopping = "Stopping",
    Stopped = "Stopped",
}

impl From<SdkRunState> for RunState {
    fn from(state: SdkRunState) -> Self {
        match state {
            SdkRunState::Idle => Self::Idle,
            SdkRunState::Running => Self::Running,
            SdkRunState::Stopping => Self::Stopping,
            SdkRunState::Stopped => Self::Stopped,
        }
    }
}
