//! The runtime's shape: the calls the driver makes on it and the thread bounds
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

/// The error a request ends in when it goes unanswered for `limit`.
pub(crate) trait Timeout {
    fn timeout(limit: Duration) -> Self;
}

/// Held while one swap's record is read, passed, and written, against anything
/// else sharing its store. On wasm32 it is a Web Lock named `name`, so tabs
/// over one IndexedDB take turns; natively it is nothing, since one process
/// owns a store.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Exclusive;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn exclusive(_name: &str) -> Exclusive {
    Exclusive
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct Exclusive(Option<js_sys::Function>);

#[cfg(target_arch = "wasm32")]
impl Drop for Exclusive {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.call0(&wasm_bindgen::JsValue::UNDEFINED);
        }
    }
}

/// Holds nothing where `navigator.locks` is missing, as in Node or outside a
/// secure context, or refuses the request. Dropping the guard settles the
/// promise the lock is held on, so a lock granted after its waiter is gone
/// is let go at once.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn exclusive(name: &str) -> Exclusive {
    use js_sys::{Array, Function, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    let locks = Reflect::get(&js_sys::global(), &"navigator".into())
        .and_then(|navigator| Reflect::get(&navigator, &"locks".into()))
        .ok()
        .filter(JsValue::is_object);
    let Some(locks) = locks else {
        return Exclusive(None);
    };
    let Some(request) = Reflect::get(&locks, &"request".into())
        .ok()
        .and_then(|request| request.dyn_into::<Function>().ok())
    else {
        return Exclusive(None);
    };

    let mut release = None;
    let held = Promise::new(&mut |resolve, _| release = Some(resolve));
    let guard = Exclusive(release);
    let mut grant = None;
    let granted = Promise::new(&mut |resolve, _| grant = Some(resolve));
    let Some(grant) = grant else {
        return guard;
    };
    let callback = Closure::once_into_js(move |_lock: JsValue| {
        let _ = grant.call0(&JsValue::UNDEFINED);
        held
    });
    let Ok(pending) = request.call2(&locks, &JsValue::from_str(name), &callback) else {
        return guard;
    };
    // `pending` settles once the lock is let go, or at once if refused.
    let pending = Promise::resolve(&pending);
    let _ =
        wasm_bindgen_futures::JsFuture::from(Promise::race(&Array::of2(&granted, &pending))).await;
    guard
}

/// `call`'s result, or its timeout once `limit` passes first, so a peer that
/// never answers costs a pass its wait and nothing more.
pub(crate) async fn bounded<T, E: Timeout>(
    limit: Duration,
    call: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    tokio::select! {
        result = call => result,
        _ = sleep(limit) => Err(E::timeout(limit)),
    }
}
