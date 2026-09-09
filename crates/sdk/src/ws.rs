//! WebSocket client for `/v1/subscribe/{user_id}`: one signal per FSM
//! transition of any swap of this wallet. A signal only says which swap moved;
//! read the state with [`HttpClient::get_swap`]. `HttpClient::signals` hands
//! it to the driver as its push channel.

use anyswap_core::api::SwapStateUpdate;
use futures_util::StreamExt;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite};

use crate::client::{ClientError, HttpClient};

#[derive(Debug, Error)]
pub enum WsError {
    #[error(transparent)]
    Client(#[from] ClientError),

    #[error("websocket: {0}")]
    Connect(#[from] tungstenite::Error),
}

/// An open subscription. It ends when the server closes it or the transport
/// fails; subscribe again to reconnect.
pub struct Subscription(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl HttpClient {
    pub async fn subscribe(&self) -> Result<Subscription, WsError> {
        let (socket, _) = connect_async(self.subscribe_url()?).await?;
        Ok(Subscription(socket))
    }
}

impl Subscription {
    /// The next update, or `None` once the subscription has ended.
    pub async fn next(&mut self) -> Option<SwapStateUpdate> {
        while let Some(message) = self.0.next().await {
            match message {
                Ok(tungstenite::Message::Text(text)) => {
                    if let Ok(update) = serde_json::from_str(&text) {
                        return Some(update);
                    }
                }
                Ok(tungstenite::Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    }
}

#[cfg(feature = "driver")]
pub use self::signals::WsSignals;

#[cfg(feature = "driver")]
mod signals {
    use async_trait::async_trait;
    use uuid::Uuid;

    use super::Subscription;
    use crate::{client::HttpClient, driver::Signals};

    /// The subscription as the driver's push channel: opened on the first
    /// `next`, dropped when it ends so the next call reopens it.
    pub struct WsSignals {
        client: HttpClient,
        subscription: Option<Subscription>,
    }

    impl HttpClient {
        pub fn signals(&self) -> WsSignals {
            WsSignals {
                client: self.clone(),
                subscription: None,
            }
        }
    }

    #[async_trait]
    impl Signals for WsSignals {
        async fn next(&mut self) -> Option<Uuid> {
            if self.subscription.is_none() {
                self.subscription = Some(self.client.subscribe().await.ok()?);
            }
            let update = self.subscription.as_mut()?.next().await;
            if update.is_none() {
                self.subscription = None;
            }
            update.map(|update| update.swap_id)
        }
    }
}
