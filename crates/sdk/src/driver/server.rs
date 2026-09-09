//! The server as the driver sees it: the calls it makes over the API, and an
//! optional push channel that only says which swap to read sooner.

use std::sync::Arc;

use anyswap_core::api::{
    CancelRequest, CreateSwapRequest, InfoResponse, RevealClaimRequest, RevealPreimageRequest,
    SwapListQuery, SwapListResponse, SwapState,
};
use async_trait::async_trait;
use uuid::Uuid;

use super::rt::{MaybeSend, MaybeSync};
use crate::client::{ClientError, HttpClient};

/// The server API as the driver uses it. Every decision is taken on what
/// `get_swap` returns and the chain confirms, so an implementation carries
/// requests and answers and never interprets them.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait ServerClient: MaybeSend + MaybeSync {
    async fn get_info(&self) -> Result<InfoResponse, ClientError>;
    async fn create_swap(&self, req: &CreateSwapRequest) -> Result<(), ClientError>;
    async fn get_swap(&self, swap_id: Uuid) -> Result<SwapState, ClientError>;
    async fn list_swaps(&self, query: &SwapListQuery) -> Result<SwapListResponse, ClientError>;
    async fn cancel(&self, swap_id: Uuid, req: &CancelRequest) -> Result<(), ClientError>;
    async fn reveal_claim(
        &self,
        swap_id: Uuid,
        req: &RevealClaimRequest,
    ) -> Result<(), ClientError>;
    async fn reveal_preimage(
        &self,
        swap_id: Uuid,
        req: &RevealPreimageRequest,
    ) -> Result<(), ClientError>;
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<C: ServerClient + ?Sized> ServerClient for Arc<C> {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        (**self).get_info().await
    }

    async fn create_swap(&self, req: &CreateSwapRequest) -> Result<(), ClientError> {
        (**self).create_swap(req).await
    }

    async fn get_swap(&self, swap_id: Uuid) -> Result<SwapState, ClientError> {
        (**self).get_swap(swap_id).await
    }

    async fn list_swaps(&self, query: &SwapListQuery) -> Result<SwapListResponse, ClientError> {
        (**self).list_swaps(query).await
    }

    async fn cancel(&self, swap_id: Uuid, req: &CancelRequest) -> Result<(), ClientError> {
        (**self).cancel(swap_id, req).await
    }

    async fn reveal_claim(
        &self,
        swap_id: Uuid,
        req: &RevealClaimRequest,
    ) -> Result<(), ClientError> {
        (**self).reveal_claim(swap_id, req).await
    }

    async fn reveal_preimage(
        &self,
        swap_id: Uuid,
        req: &RevealPreimageRequest,
    ) -> Result<(), ClientError> {
        (**self).reveal_preimage(swap_id, req).await
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl ServerClient for HttpClient {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        HttpClient::get_info(self).await
    }

    async fn create_swap(&self, req: &CreateSwapRequest) -> Result<(), ClientError> {
        HttpClient::create_swap(self, req).await
    }

    async fn get_swap(&self, swap_id: Uuid) -> Result<SwapState, ClientError> {
        HttpClient::get_swap(self, swap_id).await
    }

    async fn list_swaps(&self, query: &SwapListQuery) -> Result<SwapListResponse, ClientError> {
        HttpClient::list_swaps(self, query).await
    }

    async fn cancel(&self, swap_id: Uuid, req: &CancelRequest) -> Result<(), ClientError> {
        HttpClient::cancel(self, swap_id, req).await
    }

    async fn reveal_claim(
        &self,
        swap_id: Uuid,
        req: &RevealClaimRequest,
    ) -> Result<(), ClientError> {
        HttpClient::reveal_claim(self, swap_id, req).await
    }

    async fn reveal_preimage(
        &self,
        swap_id: Uuid,
        req: &RevealPreimageRequest,
    ) -> Result<(), ClientError> {
        HttpClient::reveal_preimage(self, swap_id, req).await
    }
}

/// A push channel naming swaps worth reading sooner. `None` means the channel
/// is down; the driver waits `poll_interval` and asks again. It paces the loops
/// and never feeds them, so an implementation may drop, repeat or invent ids
/// and change nothing but timing.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait Signals: MaybeSend {
    async fn next(&mut self) -> Option<Uuid>;
}
