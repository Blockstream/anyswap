use std::{
    collections::HashMap,
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyswap_sdk::{
    PROTOCOL_VERSION,
    api::{
        AssetPolicyResponse, CancelRequest, ChainPolicyResponse, CreateSwapRequest, ErrorCode,
        FeeEstimate, InfoResponse, LightningPolicyResponse, Pagination, Policy, RevealClaimRequest,
        RevealPreimageRequest, SwapListQuery, SwapListResponse, SwapState, TimeoutResponse,
        WebhookInfo,
    },
    auth::{AuthCore, AuthRequest},
    bitcoin::{
        Address, Amount, CompressedPublicKey, Network, OutPoint, ScriptBuf,
        hashes::{Hash as _, sha256},
        secp256k1::{Keypair, PublicKey, Secp256k1, SecretKey},
    },
    client::ClientError,
    driver::{ChainConfig, Clock, ServerClient, Timestamp},
    invoice,
    script::BitcoinSwapScript,
    transaction::bitcoin::{claim_by_preimage, refund_by_csv, sat_per_vb, spend_by_keypath},
    types::{Chain, ErrorType, NATIVE_ASSET, SwapInStatus, SwapNetwork, SwapOutStatus, SwapType},
};
use async_trait::async_trait;
use uuid::Uuid;

use super::{
    TestClock,
    chain::FakeChain,
    lightning::{Lightning, bolt11},
};

const BASE_FEE: u64 = 500;
const SERVICE_PPM: u64 = 1_000;
const MIN_AMOUNT: u64 = 5_000;
const MAX_AMOUNT: u64 = 1_000_000;
pub(super) const AWAIT_OPENING_TX: u64 = 7_200;
const INVOICE_EXPIRY: u64 = 600;
const NODE_KEY: [u8; 32] = [5; 32];
const ADDRESS_KEY: [u8; 32] = [8; 32];

/// The server as the honest plugin answers the driver, with the moves it
/// makes on its own left to the test. A move that cannot happen does nothing.
pub struct FakeServer {
    chain: Arc<FakeChain>,
    lightning: Arc<Lightning>,
    clock: TestClock,
    identity: Keypair,
    chain_config: ChainConfig,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    behavior: Behavior,
    swaps: HashMap<Uuid, Swap>,
}

/// How the server answers the driver. The default is the honest plugin.
#[derive(Clone, Copy, Debug, Default)]
pub struct Behavior {
    pub reach: Reach,
    /// Never reports its private key, even once Claimed or Canceled.
    pub withhold_key: bool,
    /// Reports a Claimed SwapOut as still awaiting the preimage.
    pub hide_claim: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    #[default]
    Up,
    /// Every call fails as a transport error.
    Down,
    /// No call is ever answered.
    Hang,
}

struct Swap {
    req: CreateSwapRequest,
    body: String,
    timestamp: i64,
    signature: String,
    swap_type: SwapType,
    key: SecretKey,
    payment_hash: [u8; 32],
    invoice: String,
    status: String,
    error_type: Option<String>,
    cancel_message: Option<String>,
    csv: u32,
    opening: Option<(OutPoint, Amount)>,
    preimage: Option<[u8; 32]>,
    user_key: Option<SecretKey>,
    settled: bool,
    created_at: Timestamp,
    transitioned_at: Timestamp,
}

impl FakeServer {
    pub fn new(
        chain: Arc<FakeChain>,
        lightning: Arc<Lightning>,
        clock: TestClock,
        identity: Keypair,
        chain_config: ChainConfig,
    ) -> Self {
        Self {
            chain,
            lightning,
            clock,
            identity,
            chain_config,
            state: Mutex::default(),
        }
    }

    /// A SwapIn's CSV, the longest the driver accepts, as the real server's
    /// configuration pairs them.
    pub fn source_csv(&self) -> u32 {
        self.chain_config.max_refund_csv
    }

    /// A SwapOut's CSV, twice the driver's `claim_margin`, as the real
    /// server's configuration pairs them.
    pub fn dest_csv(&self) -> u32 {
        2 * self.chain_config.claim_margin
    }

    pub fn behave(&self, behavior: Behavior) {
        self.state.lock().unwrap().behavior = behavior;
    }

    /// SwapOut: the user paid the hold invoice, and the server holds the HTLC.
    pub fn hold(&self, swap_id: Uuid) {
        let now = self.clock.now();
        self.swap(swap_id, |swap| {
            if swap.swap_type == SwapType::SwapOut {
                swap.set(SwapOutStatus::OpeningTxBroadcast, now);
            }
        });
    }

    /// SwapIn: pays the claim invoice for the deposit, learning its preimage.
    pub fn pay_invoice(&self, swap_id: Uuid) {
        let now = self.clock.now();
        self.swap(swap_id, |swap| {
            let deposit = self.chain.find(&swap.address(), swap.req.send_amount);
            if swap.swap_type != SwapType::SwapIn || deposit.is_none() {
                return;
            }
            swap.opening = deposit;
            swap.preimage = self.lightning.preimage(&swap.payment_hash);
            swap.set(SwapInStatus::AwaitClaimKeyRevealMessage, now);
        });
    }

    /// SwapIn: takes the deposit by keypath once it holds the user's refund
    /// key, by the preimage leaf before. `None` when it holds neither, or the
    /// chain refuses the claim.
    pub fn claim(&self, swap_id: Uuid) -> Option<String> {
        let now = self.clock.now();
        self.swap(swap_id, |swap| {
            if swap.swap_type != SwapType::SwapIn {
                return None;
            }
            let inputs = [swap.opening?];
            let rate = sat_per_vb(self.chain.fee_rate()).unwrap();
            let (script, destination) = (swap.script(), server_address());
            let tx = match (swap.user_key, swap.preimage) {
                (Some(user), _) => {
                    spend_by_keypath(&inputs, &script, &swap.key, &user, &destination, rate)
                }
                (None, Some(preimage)) => {
                    claim_by_preimage(&inputs, &script, &swap.key, &preimage, &destination, rate)
                }
                (None, None) => return None,
            };
            let txid = self.chain.submit(tx.ok()?).ok()?;
            swap.set(SwapInStatus::AwaitClaimTxConfirmation, now);
            Some(txid)
        })
    }

    /// Cancels the swap, revealing its key on a SwapIn.
    pub fn time_out(&self, swap_id: Uuid) {
        let now = self.clock.now();
        self.swap(swap_id, |swap| {
            swap.cancel(ErrorType::Timeout, "timed out", now)
        });
    }

    /// SwapOut: broadcasts the opening of `receive_amount`, once.
    pub fn open(&self, swap_id: Uuid) -> Option<String> {
        let now = self.clock.now();
        self.swap(swap_id, |swap| {
            if swap.swap_type != SwapType::SwapOut || swap.opening.is_some() {
                return None;
            }
            let address = swap.address();
            let txid = self.chain.pay(&address, swap.req.receive_amount);
            swap.opening = self.chain.find(&address, swap.req.receive_amount);
            swap.set(SwapOutStatus::AwaitPreimageRevealed, now);
            Some(txid)
        })
    }

    /// SwapOut: takes the opening back by the CSV leaf. `None` without an
    /// opening, or when the chain refuses the refund.
    pub fn refund(&self, swap_id: Uuid) -> Option<String> {
        let now = self.clock.now();
        self.swap(swap_id, |swap| {
            if swap.swap_type != SwapType::SwapOut {
                return None;
            }
            let inputs = [swap.opening?];
            let rate = sat_per_vb(self.chain.fee_rate()).unwrap();
            let tx = refund_by_csv(&inputs, &swap.script(), &swap.key, &server_address(), rate);
            let txid = self.chain.submit(tx.ok()?).ok()?;
            swap.set(SwapOutStatus::AwaitRefundTxConfirmation, now);
            Some(txid)
        })
    }

    /// SwapIn: the user's refund key, once revealed.
    pub fn refund_key(&self, swap_id: Uuid) -> Option<SecretKey> {
        self.swap(swap_id, |swap| swap.user_key)
    }

    /// SwapOut: whether the server took the payment.
    pub fn settled(&self, swap_id: Uuid) -> bool {
        self.swap(swap_id, |swap| swap.settled)
    }

    /// SwapIn: whether the server paid the invoice.
    pub fn paid(&self, swap_id: Uuid) -> bool {
        self.swap(swap_id, |swap| {
            swap.swap_type == SwapType::SwapIn && swap.preimage.is_some()
        })
    }

    fn swap<T>(&self, swap_id: Uuid, f: impl FnOnce(&mut Swap) -> T) -> T {
        let mut state = self.state.lock().unwrap();
        f(state
            .swaps
            .get_mut(&swap_id)
            .expect("a swap the server holds"))
    }

    /// The release gate held as our preimage arrived: the opening has
    /// `min_confs` and is unspent, `claim_margin` of its CSV is left, the fee
    /// estimate is within `max_fee_rate`, and a claim by the preimage to the
    /// user's address can pay it.
    fn assert_release_gate(&self, swap: &Swap, preimage: &[u8; 32]) {
        let config = &self.chain_config;
        let Some((opening, value)) = swap.opening else {
            panic!("the preimage arrived before the opening");
        };
        let confs = self.chain.confs(&opening.txid);
        let csv_left = swap.csv.saturating_sub(confs);
        let fee_rate = self.chain.fee_rate();
        assert!(
            confs >= config.min_confs,
            "the preimage arrived at {confs} confs"
        );
        assert!(
            !self.chain.spent(&opening),
            "the preimage arrived with the opening spent"
        );
        assert!(
            csv_left >= config.claim_margin,
            "the preimage arrived with {csv_left} blocks of CSV left"
        );
        assert!(
            fee_rate <= config.max_fee_rate,
            "the preimage arrived at {fee_rate} sat/vB"
        );
        let address = swap.req.user_claim_address.as_deref().unwrap();
        let destination = Address::from_str(address)
            .unwrap()
            .assume_checked()
            .script_pubkey();
        let rate = sat_per_vb(fee_rate).unwrap();
        let claim = claim_by_preimage(
            &[(opening, value)],
            &swap.script(),
            &swap.key,
            preimage,
            &destination,
            rate,
        );
        assert!(
            claim.is_ok(),
            "the preimage arrived when its claim could not pay {fee_rate} sat/vB"
        );
    }

    async fn reach(&self) -> Result<(), ClientError> {
        let reach = self.state.lock().unwrap().behavior.reach;
        match reach {
            Reach::Up => Ok(()),
            Reach::Down => Err(ClientError::Transport("down".into())),
            Reach::Hang => std::future::pending().await,
        }
    }

    fn create(&self, req: &CreateSwapRequest) -> Swap {
        let body = serde_json::to_string(req).unwrap();
        let req: CreateSwapRequest = serde_json::from_str(&body).unwrap();
        let now = self.clock.now();
        let timestamp = now as i64;
        let signature = AuthCore::sign_request(
            &AuthRequest {
                method: "POST",
                target: "/v1/swap",
                body: body.as_bytes(),
                swap_id: &req.swap_id.to_string(),
                timestamp,
            },
            &self.identity,
        );
        let key = sha256::Hash::hash(req.swap_id.as_bytes()).to_byte_array();
        let (swap_type, payment_hash, invoice, status) = match &req.claim_invoice {
            Some(claim_invoice) => (
                SwapType::SwapIn,
                invoice::payment_hash(claim_invoice).unwrap(),
                claim_invoice.clone(),
                SwapInStatus::AwaitOpeningTxConfirmation.to_string(),
            ),
            None => {
                let payment_hash = hex::decode(req.payment_hash.as_deref().unwrap()).unwrap();
                let payment_hash: [u8; 32] = payment_hash.try_into().unwrap();
                let invoice = bolt11(payment_hash, req.send_amount, now, INVOICE_EXPIRY, NODE_KEY);
                let status = SwapOutStatus::AwaitClaimInvoicePay.to_string();
                (SwapType::SwapOut, payment_hash, invoice, status)
            }
        };
        let csv = match swap_type {
            SwapType::SwapIn => self.source_csv(),
            _ => self.dest_csv(),
        };
        Swap {
            req,
            body,
            timestamp,
            signature: hex::encode(signature.as_ref()),
            swap_type,
            key: SecretKey::from_slice(&key).unwrap(),
            payment_hash,
            invoice,
            status,
            error_type: None,
            cancel_message: None,
            csv,
            opening: None,
            preimage: None,
            user_key: None,
            settled: false,
            created_at: now,
            transitioned_at: now,
        }
    }
}

#[async_trait]
impl ServerClient for FakeServer {
    async fn get_info(&self) -> Result<InfoResponse, ClientError> {
        self.reach().await?;
        Ok(self.info())
    }

    async fn create_swap(&self, req: &CreateSwapRequest) -> Result<(), ClientError> {
        self.reach().await?;
        let swap = self.create(req);
        let mut state = self.state.lock().unwrap();
        state.swaps.insert(swap.req.swap_id, swap);
        Ok(())
    }

    async fn get_swap(&self, swap_id: Uuid) -> Result<SwapState, ClientError> {
        self.reach().await?;
        let state = self.state.lock().unwrap();
        let swap = state.swaps.get(&swap_id).ok_or_else(not_found)?;
        Ok(swap.state(&state.behavior))
    }

    async fn list_swaps(&self, query: &SwapListQuery) -> Result<SwapListResponse, ClientError> {
        self.reach().await?;
        let state = self.state.lock().unwrap();
        let mut swaps: Vec<_> = state
            .swaps
            .values()
            .filter(|swap| swap.req.user_id == query.user_id)
            .collect();
        swaps.sort_by_key(|swap| (swap.created_at, swap.req.swap_id));
        let total = swaps.len() as u32;
        let page: Vec<_> = swaps
            .into_iter()
            .skip(query.offset as usize)
            .take(query.limit as usize)
            .map(|swap| match query.with_create_request {
                true => swap.state(&state.behavior),
                false => swap.state(&state.behavior).without_create_request(),
            })
            .collect();
        Ok(SwapListResponse {
            pagination: Pagination {
                total,
                limit: query.limit,
                offset: query.offset,
                has_next: query.offset + (page.len() as u32) < total,
            },
            swaps: page,
        })
    }

    async fn cancel(&self, swap_id: Uuid, req: &CancelRequest) -> Result<(), ClientError> {
        self.reach().await?;
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap();
        let swap = state.swaps.get_mut(&swap_id).ok_or_else(not_found)?;
        let open = match swap.swap_type {
            SwapType::SwapIn => SwapInStatus::AwaitOpeningTxConfirmation.to_string(),
            _ => SwapOutStatus::AwaitClaimInvoicePay.to_string(),
        };
        if swap.status == SwapInStatus::Canceled.to_string() {
            return Ok(());
        }
        if swap.status != open {
            return Err(rejected());
        }
        swap.cancel(ErrorType::UserCanceled, &req.cancel_message, now);
        Ok(())
    }

    async fn reveal_claim(
        &self,
        swap_id: Uuid,
        req: &RevealClaimRequest,
    ) -> Result<(), ClientError> {
        self.reach().await?;
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap();
        let swap = state.swaps.get_mut(&swap_id).ok_or_else(not_found)?;
        assert!(
            swap.preimage.is_some(),
            "our refund key arrived before the server paid the invoice"
        );
        if swap.status != SwapInStatus::AwaitClaimKeyRevealMessage.to_string() {
            return Err(rejected());
        }
        swap.user_key = Some(SecretKey::from_str(&req.source_refund_privkey).unwrap());
        swap.set(SwapInStatus::AwaitClaimTxConfirmation, now);
        Ok(())
    }

    async fn reveal_preimage(
        &self,
        swap_id: Uuid,
        req: &RevealPreimageRequest,
    ) -> Result<(), ClientError> {
        self.reach().await?;
        let now = self.clock.now();
        let mut state = self.state.lock().unwrap();
        let swap = state.swaps.get_mut(&swap_id).ok_or_else(not_found)?;
        let preimage: [u8; 32] = hex::decode(&req.preimage).unwrap().try_into().unwrap();
        assert_eq!(
            sha256::Hash::hash(&preimage).to_byte_array(),
            swap.payment_hash
        );
        self.assert_release_gate(swap, &preimage);
        if swap.status != SwapOutStatus::AwaitPreimageRevealed.to_string() {
            return Err(rejected());
        }
        swap.preimage = Some(preimage);
        swap.settled = true;
        swap.set(SwapOutStatus::Claimed, now);
        Ok(())
    }
}

impl Swap {
    fn set(&mut self, status: impl ToString, now: Timestamp) {
        self.status = status.to_string();
        self.transitioned_at = now;
    }

    fn cancel(&mut self, error_type: ErrorType, message: &str, now: Timestamp) {
        self.error_type = Some(error_type.to_string());
        self.cancel_message = Some(message.into());
        self.set(SwapInStatus::Canceled, now);
    }

    fn script(&self) -> BitcoinSwapScript {
        let server = self.key.public_key(&Secp256k1::new()).x_only_public_key().0;
        let user = |hex: &Option<String>| {
            let key = PublicKey::from_str(hex.as_deref().unwrap()).unwrap();
            key.x_only_public_key().0
        };
        let (claim, refund) = match self.swap_type {
            SwapType::SwapIn => (server, user(&self.req.source_refund_pubkey)),
            _ => (user(&self.req.dest_claim_pubkey), server),
        };
        BitcoinSwapScript::new(claim, refund, self.payment_hash, self.csv).unwrap()
    }

    fn address(&self) -> String {
        self.script().address(SwapNetwork::Regtest)
    }

    /// Its key goes out as the real server's does: a SwapIn's once it is
    /// Canceled, a SwapOut's once it is Claimed.
    fn reveals_key(&self) -> bool {
        match self.swap_type {
            SwapType::SwapIn => self.status == SwapInStatus::Canceled.to_string(),
            _ => self.status == SwapOutStatus::Claimed.to_string(),
        }
    }

    fn state(&self, behavior: &Behavior) -> SwapState {
        let swap_in = self.swap_type == SwapType::SwapIn;
        let pubkey = Some(self.key.public_key(&Secp256k1::new()).to_string());
        let privkey = (self.reveals_key() && !behavior.withhold_key)
            .then(|| hex::encode(self.key.secret_bytes()));
        let status = match behavior.hide_claim && self.status == SwapOutStatus::Claimed.to_string()
        {
            true => SwapOutStatus::AwaitPreimageRevealed.to_string(),
            false => self.status.clone(),
        };
        let txid = self.opening.map(|(outpoint, _)| outpoint.txid.to_string());
        let vout = self.opening.map(|(outpoint, _)| outpoint.vout);
        SwapState {
            protocol_version: PROTOCOL_VERSION.into(),
            swap_id: self.req.swap_id,
            swap_type: self.swap_type,
            status,
            receive_amount: self.req.receive_amount,
            send_amount: self.req.send_amount,
            base_fee: BASE_FEE,
            service_fee: self.req.send_amount - self.req.receive_amount - BASE_FEE,
            invoice_fee: None,
            opening_tx_fee: None,
            claim_tx_fee: None,
            refund_tx_fee: None,
            payment_hash: hex::encode(self.payment_hash),
            preimage: self.preimage.filter(|_| swap_in).map(hex::encode),
            transitioned_at: self.transitioned_at,
            created_at: self.created_at,
            error_type: self.error_type.clone(),
            cancel_message: self.cancel_message.clone(),
            source_chain: swap_in.then_some(Chain::Bitcoin),
            source_asset: swap_in.then(|| NATIVE_ASSET.into()),
            source_csv: swap_in.then_some(self.csv),
            source_txid: txid.clone().filter(|_| swap_in),
            source_vout: vout.filter(|_| swap_in),
            source_refund_pubkey: self.req.source_refund_pubkey.clone(),
            source_claim_pubkey: pubkey.clone().filter(|_| swap_in),
            source_claim_privkey: privkey.clone().filter(|_| swap_in),
            source_user_blinding_pubkey: None,
            source_service_blinding_pubkey: None,
            dest_chain: (!swap_in).then_some(Chain::Bitcoin),
            dest_asset: (!swap_in).then(|| NATIVE_ASSET.into()),
            dest_csv: (!swap_in).then_some(self.csv),
            dest_txid: txid.filter(|_| !swap_in),
            dest_vout: vout.filter(|_| !swap_in),
            dest_refund_pubkey: pubkey.filter(|_| !swap_in),
            dest_claim_pubkey: self.req.dest_claim_pubkey.clone(),
            dest_refund_privkey: privkey.filter(|_| !swap_in),
            user_claim_address: self.req.user_claim_address.clone(),
            user_refund_address: self.req.user_refund_address.clone(),
            server_claim_address: None,
            server_refund_address: None,
            source_claim_txid: None,
            dest_refund_txid: None,
            dest_user_blinding_pubkey: None,
            dest_service_blinding_pubkey: None,
            claim_invoice: Some(self.invoice.clone()),
            create_request_body: Some(self.body.clone()),
            create_request_timestamp: Some(self.timestamp),
            create_request_signature: Some(self.signature.clone()),
        }
    }
}

impl FakeServer {
    fn info(&self) -> InfoResponse {
        let asset = AssetPolicyResponse {
            asset_id: NATIVE_ASSET.into(),
            min_amount: MIN_AMOUNT,
            max_amount: MAX_AMOUNT,
        };
        let bitcoin = ChainPolicyResponse {
            source_csv: self.source_csv(),
            dest_csv: self.dest_csv(),
            confs: self.chain_config.min_confs,
            block_time: self.chain_config.block_time as u32,
            timeouts: TimeoutResponse {
                await_opening_tx: AWAIT_OPENING_TX,
                await_claim_key_reveal_message: 3_600,
            },
            service_ppm: SERVICE_PPM,
            min_base_fee_multiplier: 1,
            asset: HashMap::from([(NATIVE_ASSET.to_owned(), asset)]),
        };
        InfoResponse {
            online: true,
            policy: Policy {
                protocol_version: PROTOCOL_VERSION.into(),
                network: SwapNetwork::Regtest,
                invoice_expiry: INVOICE_EXPIRY,
                chain: HashMap::from([(Chain::Bitcoin, bitcoin)]),
                lightning: LightningPolicyResponse {
                    min_amount: MIN_AMOUNT,
                    max_amount: MAX_AMOUNT,
                    service_ppm: SERVICE_PPM,
                },
            },
            fee_estimates: HashMap::from([(
                Chain::Bitcoin,
                FeeEstimate {
                    source: BASE_FEE,
                    dest: BASE_FEE,
                },
            )]),
            quote_id: Some(Uuid::from_u128(1)),
            quote_ttl: 600,
            webhook: WebhookInfo::default(),
        }
    }
}

fn server_address() -> ScriptBuf {
    let key = SecretKey::from_slice(&ADDRESS_KEY).unwrap();
    let pubkey = CompressedPublicKey(key.public_key(&Secp256k1::new()));
    Address::p2wpkh(&pubkey, Network::Regtest).script_pubkey()
}

fn rejected() -> ClientError {
    ClientError::Api {
        status: 409,
        message: "transition rejected".into(),
        code: Some(ErrorCode::TransitionRejected),
    }
}

fn not_found() -> ClientError {
    ClientError::Api {
        status: 404,
        message: "swap not found".into(),
        code: Some(ErrorCode::SwapNotFound),
    }
}
