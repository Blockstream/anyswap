//! HTTP client for the AnySwap server API.
//!
//! The server is the responder; the user always initiates. POST methods trigger
//! server-side transitions and return no body - the resulting state is always
//! read back via [`HttpClient::get_swap`] and parsed into a typed
//! [`SwapIn`](crate::swap_in::SwapIn) via `TryFrom`. Poll on a timer; WebSocket
//! signals only shorten the wait, they never replace the snapshot.
//!
//! ## Swap-in (on-chain to Lightning)
//!
//! After [`HttpClient::create_swap`], poll with [`HttpClient::get_swap`]:
//!
//! 1. `AwaitOpeningTxConfirmation` - deposit to the swap script address; the server watches the
//!    address for the opening tx and its confirmations.
//! 3. `PayInvoice` - the server pays the user's `claim_invoice`.
//! 4. `AwaitClaimKeyRevealMessage` - the invoice is paid; check that with
//!    [`SwapIn::verify_payment`](crate::swap_in::SwapIn::verify_payment), then reveal the refund
//!    key via [`HttpClient::reveal_claim`] so the server can claim cooperatively via the keypath
//!    instead of the preimage script path.
//! 5. `AwaitClaimTxConfirmation` then `Claimed` - swap finished.
//!
//! [`HttpClient::cancel`] before a successful claim results in `Canceled`,
//! where the server reveals its claim privkey so a deposited user refunds
//! immediately via [`SwapIn::refund_by_keypath`](crate::swap_in::SwapIn::refund_by_keypath).
//!
//! ## Swap-out (Lightning to on-chain)
//!
//! The user holds the preimage. After [`HttpClient::create_swap`], poll with
//! [`HttpClient::get_swap`] and parse into a typed
//! [`SwapOut`](crate::swap_out::SwapOut):
//!
//! 1. `AwaitClaimInvoicePay` - pay the held `claim_invoice` over Lightning.
//! 2. `OpeningTxBroadcast` - the server funds the destination opening tx.
//! 3. `AwaitPreimageRevealed` - once the opening tx has enough confirmations, claim either
//!    cooperatively (reveal the preimage off-chain via [`HttpClient::reveal_preimage`], then
//!    keypath spend) or unilaterally on-chain via
//!    [`SwapOut::claim_by_preimage`](crate::swap_out::SwapOut::claim_by_preimage).
//! 4. `Claimed` - the held invoice is settled. After an off-chain reveal the server exposes its
//!    refund key; finish with
//!    [`SwapOut::claim_by_keypath`](crate::swap_out::SwapOut::claim_by_keypath).
//!
//! If the user never claims, the server refunds the destination output after
//! the CSV timeout (`AwaitRefundTxConfirmation` then `Refunded`). There is no
//! way to abort a swap-out once its opening tx is broadcast.

use anyswap_core::{
    api::{
        CancelRequest, CreateSwapRequest, ErrorCode, InfoResponse, RevealClaimRequest,
        RevealPreimageRequest, SwapListQuery, SwapListResponse, SwapState,
    },
    auth::{AuthCore, AuthRequest, IDENTITY_PUBKEY_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER},
    utils::now,
};
use bitcoin::{
    PrivateKey,
    hashes::{Hash, sha256},
    secp256k1::{Keypair, Secp256k1},
};
use reqwest::{
    Method,
    header::{CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ClientError {
    /// The server could not be reached, or the exchange broke before an
    /// answer.
    #[error("transport: {0}")]
    Transport(String),

    #[error("server ({status}): {message}")]
    Api {
        status: u16,
        message: String,
        /// Machine-readable error code, absent on responses without a known one.
        code: Option<ErrorCode>,
    },

    #[error("invalid auth header value: {0}")]
    Header(#[from] reqwest::header::InvalidHeaderValue),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("missing identity private key")]
    MissingPrivateKey,
}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        Self::Transport(e.to_string())
    }
}

impl ClientError {
    /// The server's FSM refused a message it is past; the outcome is the same
    /// as having delivered it.
    pub fn transition_rejected(&self) -> bool {
        matches!(
            self,
            Self::Api {
                code: Some(ErrorCode::TransitionRejected),
                ..
            }
        )
    }
}

/// HTTP client for the AnySwap server API. See the [module docs](self) for swap flows.
#[derive(Clone)]
pub struct HttpClient {
    http: reqwest::Client,
    base_url: String,
    private_key: Option<PrivateKey>,
}

impl HttpClient {
    pub fn new(base_url: &str, private_key: Option<PrivateKey>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            private_key,
        }
    }

    pub fn set_private_key(&mut self, private_key: PrivateKey) -> &mut Self {
        self.private_key = Some(private_key);
        self
    }

    /// The signed `/v1/subscribe/{user_id}` WebSocket URL, auth in the query.
    pub fn subscribe_url(&self) -> Result<String, ClientError> {
        let identity_private_key = self
            .private_key
            .as_ref()
            .ok_or(ClientError::MissingPrivateKey)?;
        let keypair = identity_keypair(identity_private_key);
        let pubkey = keypair.public_key();
        let user_id = sha256::Hash::hash(&pubkey.serialize()).to_string();
        let target = format!("/v1/subscribe/{user_id}");
        let timestamp = now() as i64;
        let signature = AuthCore::sign_request(
            &AuthRequest {
                method: Method::GET.as_str(),
                target: &target,
                body: &[],
                swap_id: "",
                timestamp,
            },
            &keypair,
        );
        let base = self.base_url.replacen("http", "ws", 1);
        Ok(format!(
            "{base}{target}?{IDENTITY_PUBKEY_HEADER}={}&{TIMESTAMP_HEADER}={timestamp}&{SIGNATURE_HEADER}={}",
            hex::encode(pubkey.serialize()),
            hex::encode(signature.as_ref()),
        ))
    }

    /// Fetches server info: protocol version, network, and per-chain policies
    /// (amount limits, confirmation requirements, swap_fee rates).
    pub async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        let resp = self
            .http
            .get(format!("{}/v1/info", self.base_url))
            .send()
            .await?;
        Ok(check(resp).await?.json().await?)
    }

    /// Initiates a swap. The server may cancel the swap right after creation;
    /// the actual state is discovered via [`HttpClient::get_swap`].
    pub async fn create_swap(&self, req: &CreateSwapRequest) -> Result<(), ClientError> {
        let path = "/v1/swap";
        let body = serde_json::to_vec(req)?;
        let identity_private_key = self
            .private_key
            .as_ref()
            .ok_or(ClientError::MissingPrivateKey)?;
        let identity_keypair = identity_keypair(identity_private_key);
        let headers = self.prepare_headers(
            &req.swap_id.to_string(),
            &Method::POST,
            path,
            &body,
            &identity_keypair,
        )?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        check(resp).await?;
        Ok(())
    }

    /// Polls the current state of a swap. Convert the response to a typed
    /// [`SwapIn`](crate::swap_in::SwapIn) via [`TryFrom`].
    pub async fn get_swap(&self, swap_id: Uuid) -> Result<SwapState, ClientError> {
        let path = format!("/v1/swap/{swap_id}");
        let identity_private_key = self
            .private_key
            .as_ref()
            .ok_or(ClientError::MissingPrivateKey)?;
        let identity_keypair = identity_keypair(identity_private_key);
        let headers = self.prepare_headers(
            &swap_id.to_string(),
            &Method::GET,
            &path,
            &[],
            &identity_keypair,
        )?;

        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .headers(headers)
            .send()
            .await?;
        Ok(check(resp).await?.json().await?)
    }

    /// Fetches the paginated swap history for a user.
    pub async fn list_swaps(&self, query: &SwapListQuery) -> Result<SwapListResponse, ClientError> {
        let path = "/v1/swap";
        let identity_private_key = self
            .private_key
            .as_ref()
            .ok_or(ClientError::MissingPrivateKey)?;
        let identity_keypair = identity_keypair(identity_private_key);

        let mut request = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .query(query)
            .build()?;
        let target = match request.url().query() {
            Some(query) => format!("{path}?{query}"),
            None => path.to_owned(),
        };
        let headers = self.prepare_headers("", &Method::GET, &target, &[], &identity_keypair)?;
        request.headers_mut().extend(headers);

        let resp = self.http.execute(request).await?;
        Ok(check(resp).await?.json().await?)
    }

    /// Requests swap cancellation. The outcome is observed via
    /// [`HttpClient::get_swap`]: `Canceled` reveals the server's claim privkey
    /// for an immediate keypath refund of any deposit.
    pub async fn cancel(&self, swap_id: Uuid, req: &CancelRequest) -> Result<(), ClientError> {
        self.post_protected(swap_id, "/cancel", req).await
    }

    /// Reveals the user's refund privkey after the claim invoice was paid
    /// (swap-in only), letting the server claim cooperatively via the keypath
    /// instead of the preimage script path.
    pub async fn reveal_claim(
        &self,
        swap_id: Uuid,
        req: &RevealClaimRequest,
    ) -> Result<(), ClientError> {
        self.post_protected(swap_id, "/reveal/claim", req).await
    }

    /// Reveals the preimage off-chain (swap-out only) instead of claiming via
    /// the script path. The server settles the held invoice and exposes its
    /// refund privkey, letting the user finish with a cheap cooperative keypath
    /// claim. Reveal only after the opening tx has the required confirmations.
    pub async fn reveal_preimage(
        &self,
        swap_id: Uuid,
        req: &RevealPreimageRequest,
    ) -> Result<(), ClientError> {
        self.post_protected(swap_id, "/reveal/preimage", req).await
    }

    async fn post_protected(
        &self,
        swap_id: Uuid,
        path_suffix: &str,
        req: &impl serde::Serialize,
    ) -> Result<(), ClientError> {
        let body = serde_json::to_vec(req)?;
        let path = format!("/v1/swap/{swap_id}{path_suffix}");
        let identity_private_key = self
            .private_key
            .as_ref()
            .ok_or(ClientError::MissingPrivateKey)?;

        let identity_keypair = identity_keypair(identity_private_key);
        let headers = self.prepare_headers(
            &swap_id.to_string(),
            &Method::POST,
            &path,
            &body,
            &identity_keypair,
        )?;

        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await?;
        check(resp).await?;
        Ok(())
    }

    // Builds the exact auth payload the plugin middleware verifies.
    fn prepare_headers(
        &self,
        swap_id: &str,
        method: &Method,
        target: &str,
        raw_body: &[u8],
        identity_keypair: &Keypair,
    ) -> Result<HeaderMap, ClientError> {
        let timestamp = now() as i64;
        let pubkey = identity_keypair.public_key();
        let signature = AuthCore::sign_request(
            &AuthRequest {
                method: method.as_str(),
                target,
                body: raw_body,
                swap_id,
                timestamp,
            },
            identity_keypair,
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            IDENTITY_PUBKEY_HEADER,
            HeaderValue::from_str(&hex::encode(pubkey.serialize()))?,
        );
        headers.insert(
            TIMESTAMP_HEADER,
            HeaderValue::from_str(&timestamp.to_string())?,
        );
        headers.insert(
            SIGNATURE_HEADER,
            HeaderValue::from_str(&hex::encode(signature.as_ref()))?,
        );

        Ok(headers)
    }
}

// decode the private key to keypair
fn identity_keypair(identity_private_key: &PrivateKey) -> Keypair {
    let secp = Secp256k1::new();
    Keypair::from_secret_key(&secp, &identity_private_key.inner)
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response, ClientError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }

    #[derive(Deserialize)]
    struct ErrorBody {
        message: String,
        #[serde(default)]
        code: Option<String>,
    }

    let (message, code) = match resp.json::<ErrorBody>().await {
        Ok(body) => (body.message, body.code.and_then(|c| c.parse().ok())),
        Err(_) => (
            status
                .canonical_reason()
                .unwrap_or("unknown error")
                .to_string(),
            None,
        ),
    };
    Err(ClientError::Api {
        status: status.as_u16(),
        message,
        code,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        time::Duration,
    };

    use anyswap_core::types::Chain;
    use bitcoin::{
        Network,
        hashes::{Hash, sha256},
        secp256k1::{SecretKey, schnorr::Signature},
    };

    use super::*;

    struct RecordedRequest {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    fn test_keypair() -> Keypair {
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(&[1; 32]).unwrap();
        Keypair::from_secret_key(&secp, &secret)
    }

    fn test_identity_private_key() -> PrivateKey {
        PrivateKey::new(SecretKey::from_slice(&[1; 32]).unwrap(), Network::Regtest)
    }

    fn read_request(stream: &mut TcpStream) -> RecordedRequest {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut data = Vec::new();
        let mut buf = [0; 1024];
        let header_end = loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "client closed connection before sending headers");
            data.extend_from_slice(&buf[..n]);

            if let Some(pos) = data.windows(4).position(|window| window == b"\r\n\r\n") {
                break pos;
            }
        };

        let headers = String::from_utf8(data[..header_end].to_vec()).unwrap();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let body_start = header_end + 4;
        let total_len = body_start + content_length;

        while data.len() < total_len {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "client closed connection before sending body");
            data.extend_from_slice(&buf[..n]);
        }

        let mut lines = headers.lines();
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let headers = lines
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.to_ascii_lowercase(), value.trim().to_string()))
            })
            .collect();

        RecordedRequest {
            method,
            path,
            headers,
            body: data[body_start..total_len].to_vec(),
        }
    }

    fn auth_header<'a>(headers: &'a HashMap<String, String>, name: &str) -> &'a str {
        headers.get(name).map(String::as_str).unwrap()
    }

    fn signature_and_timestamp(headers: &HeaderMap) -> (Signature, i64) {
        let signature_hex = headers.get(SIGNATURE_HEADER).unwrap().to_str().unwrap();
        let timestamp = headers.get(TIMESTAMP_HEADER).unwrap().to_str().unwrap();

        (
            Signature::from_slice(&hex::decode(signature_hex).unwrap()).unwrap(),
            timestamp.parse().unwrap(),
        )
    }

    #[test]
    fn prepare_headers_signs_the_target_including_its_query() {
        let client = HttpClient::new("http://localhost:8083", None);
        let keypair = test_keypair();
        let pubkey = keypair.public_key();
        let target = "/v1/swap?user_id=alice&limit=20";

        let headers = client
            .prepare_headers("", &Method::GET, target, &[], &keypair)
            .unwrap();
        let (signature, timestamp) = signature_and_timestamp(&headers);
        let signed = AuthRequest {
            method: Method::GET.as_str(),
            target,
            body: &[],
            swap_id: "",
            timestamp,
        };

        assert!(AuthCore::validate_signature(
            &AuthCore::request_digest(&signed, &pubkey),
            &signature,
            &pubkey
        ));

        let widened = AuthRequest {
            target: "/v1/swap?user_id=alice&limit=500",
            ..signed
        };
        assert!(!AuthCore::validate_signature(
            &AuthCore::request_digest(&widened, &pubkey),
            &signature,
            &pubkey
        ));
    }

    #[test]
    fn prepare_headers_sets_verifiable_signature() {
        let client = HttpClient::new("http://localhost:8083", None);
        let keypair = test_keypair();
        let pubkey = keypair.public_key();

        let headers = client
            .prepare_headers("swap-1", &Method::GET, "/v1/swap/swap-1", &[], &keypair)
            .unwrap();

        assert_eq!(
            headers.get(IDENTITY_PUBKEY_HEADER).unwrap(),
            hex::encode(pubkey.serialize()).as_str()
        );
        let (signature, timestamp) = signature_and_timestamp(&headers);
        assert!(timestamp > 0);

        let signed = AuthRequest {
            method: Method::GET.as_str(),
            target: "/v1/swap/swap-1",
            body: &[],
            swap_id: "swap-1",
            timestamp,
        };

        assert!(AuthCore::validate_signature(
            &AuthCore::request_digest(&signed, &pubkey),
            &signature,
            &pubkey
        ));
    }

    #[tokio::test]
    async fn create_swap_signature_verifies_against_body_swap_id() {
        let swap_id = Uuid::from_u128(0x1234567890abcdef1234567890abcdef);
        let pubkey = test_keypair().public_key();
        let req = CreateSwapRequest {
            protocol_version: "3.0.0".into(),
            swap_id,
            user_id: sha256::Hash::hash(&pubkey.serialize()).to_string(),
            source_chain: Some(Chain::Bitcoin),
            source_asset_id: Some(anyswap_core::types::NATIVE_ASSET.to_string()),
            dest_chain: None,
            dest_asset_id: None,
            receive_amount: 10_000,
            swap_fee_limit: 1_000,
            quote_id: None,
            claim_invoice: Some("lnbc1test".to_string()),
            source_refund_pubkey: Some(hex::encode(pubkey.serialize())),
            source_blinding_pubkey: None,
            dest_claim_pubkey: None,
            dest_blinding_pubkey: None,
            user_claim_address: None,
            user_refund_address: None,
            payment_hash: None,
            webhook: None,
        };
        let expected_body = serde_json::to_vec(&req).unwrap();
        let expected_user_id = req.user_id.clone();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);

            assert_eq!(request.method, Method::POST.as_str());
            assert_eq!(request.path, "/v1/swap");
            assert_eq!(request.body, expected_body);

            let auth_params = anyswap_core::auth::AuthParams::from_fields(
                auth_header(&request.headers, IDENTITY_PUBKEY_HEADER),
                auth_header(&request.headers, TIMESTAMP_HEADER),
                auth_header(&request.headers, SIGNATURE_HEADER),
            )
            .unwrap();
            assert!(AuthCore::validate_pubkey(
                &auth_params.pubkey,
                &expected_user_id
            ));

            let signed = AuthRequest {
                method: Method::POST.as_str(),
                target: &request.path,
                body: &request.body,
                swap_id: &swap_id.to_string(),
                timestamp: auth_params.timestamp,
            };
            assert!(AuthCore::validate_signature(
                &AuthCore::request_digest(&signed, &auth_params.pubkey),
                &auth_params.signature,
                &auth_params.pubkey
            ));

            let wrong_swap_id = AuthRequest {
                swap_id: "",
                ..signed
            };
            assert!(!AuthCore::validate_signature(
                &AuthCore::request_digest(&wrong_swap_id, &auth_params.pubkey),
                &auth_params.signature,
                &auth_params.pubkey
            ));

            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
        });

        let mut client = HttpClient::new(&base_url, None);
        client.set_private_key(test_identity_private_key());
        let result = client.create_swap(&req).await;
        let server_result = server.join();

        assert!(result.is_ok(), "{result:?}");
        server_result.unwrap();
    }
}
