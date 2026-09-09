//! The runtime's shape: the two calls the driver makes and the thread bounds
//! it asks for. Tokio and `Send` natively, the browser's own and no bounds on
//! wasm32, so the loop is written once. Nothing else in the driver may name
//! a runtime.

use std::{future::Future, time::Duration};

/// `Send` everywhere but wasm32, where nothing crosses a thread and the
/// browser's objects cannot.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// `Sync` on the same terms as [`MaybeSend`].
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync + ?Sized> MaybeSync for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSync for T {}

/// Spawns a future on the runtime the driver runs on: tokio natively, the
/// browser's event loop on wasm32. What `Driver::run` is handed to, written
/// once for both targets.
pub fn spawn(future: impl Future<Output = ()> + MaybeSend + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::spawn(future);
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(future);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    /// The global `setTimeout`, so this works in a worker as well as a window.
    #[wasm_bindgen(js_name = setTimeout)]
    fn set_timeout(handler: &js_sys::Function, millis: i32);
}

#[cfg(target_arch = "wasm32")]
pub(crate) async fn sleep(duration: Duration) {
    let millis = duration.as_millis().min(i32::MAX as u128) as i32;
    let promise = js_sys::Promise::new(&mut |resolve, _| set_timeout(&resolve, millis));
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
