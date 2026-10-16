//! What a driver call fails with, and the code a page picks its handling by.

use anyswap_sdk::{api::ErrorCode, client::ClientError, driver::DriverError as SdkDriverError};
use wasm_bindgen::prelude::*;

/// What a driver call failed with: a `code` to act on, a `message` to show.
/// `retryable` means the chain or server was unreachable, or the server asked
/// to slow down, and the next call sees the same world.
#[wasm_bindgen]
#[derive(Clone)]
pub struct DriverError {
    message: String,
    code: DriverErrorCode,
    http_status: Option<u16>,
    retryable: bool,
}

#[wasm_bindgen]
impl DriverError {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }

    /// What kind of failure. `Internal` is a fault on the server, or on
    /// this side what should never happen.
    #[wasm_bindgen(getter)]
    pub fn code(&self) -> DriverErrorCode {
        self.code
    }

    /// The HTTP status the server answered with, `undefined` when the failure
    /// is not an answer from the server.
    #[wasm_bindgen(getter, js_name = httpStatus)]
    pub fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    #[wasm_bindgen(getter)]
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    #[wasm_bindgen(js_name = toString)]
    pub fn to_string_js(&self) -> String {
        self.message.clone()
    }
}

impl From<SdkDriverError> for DriverError {
    fn from(e: SdkDriverError) -> Self {
        Self::from(&e)
    }
}

impl From<&SdkDriverError> for DriverError {
    fn from(e: &SdkDriverError) -> Self {
        let http_status = match e {
            SdkDriverError::Server(ClientError::Api { status, .. }) => Some(*status),
            _ => None,
        };
        Self {
            message: e.to_string(),
            code: e.into(),
            http_status,
            retryable: e.retryable(),
        }
    }
}

/// The kind of a `DriverError`. The server's codes keep the server's names;
/// the rest name what failed on this side.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum DriverErrorCode {
    Internal = "Internal",
    BadRequest = "BadRequest",
    NotImplemented = "NotImplemented",
    QuoteExpired = "QuoteExpired",
    SwapNotFound = "SwapNotFound",
    SwapAlreadyExists = "SwapAlreadyExists",
    TransitionRejected = "TransitionRejected",
    PolicyError = "PolicyError",
    TooManyConnections = "TooManyConnections",
    Unavailable = "Unavailable",
    Forbidden = "Forbidden",
    PayloadTooLarge = "PayloadTooLarge",
    InsufficientLiquidity = "InsufficientLiquidity",
    Transport = "Transport",
    Chain = "Chain",
    Store = "Store",
    Wallet = "Wallet",
    Destination = "Destination",
    Refused = "Refused",
    Unsigned = "Unsigned",
    BadSignature = "BadSignature",
    ChainNotConfigured = "ChainNotConfigured",
    UnknownSwap = "UnknownSwap",
    Unsupported = "Unsupported",
    WrongSwapType = "WrongSwapType",
    NotQuotable = "NotQuotable",
    AmountOutOfRange = "AmountOutOfRange",
    /// A bad argument to a binding.
    Invalid = "Invalid",
}

impl From<ErrorCode> for DriverErrorCode {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::Internal => Self::Internal,
            ErrorCode::BadRequest => Self::BadRequest,
            ErrorCode::NotImplemented => Self::NotImplemented,
            ErrorCode::QuoteExpired => Self::QuoteExpired,
            ErrorCode::SwapNotFound => Self::SwapNotFound,
            ErrorCode::SwapAlreadyExists => Self::SwapAlreadyExists,
            ErrorCode::TransitionRejected => Self::TransitionRejected,
            ErrorCode::PolicyError => Self::PolicyError,
            ErrorCode::TooManyConnections => Self::TooManyConnections,
            ErrorCode::Unavailable => Self::Unavailable,
            ErrorCode::Forbidden => Self::Forbidden,
            ErrorCode::PayloadTooLarge => Self::PayloadTooLarge,
            ErrorCode::InsufficientLiquidity => Self::InsufficientLiquidity,
        }
    }
}

impl From<&SdkDriverError> for DriverErrorCode {
    fn from(e: &SdkDriverError) -> Self {
        match e {
            SdkDriverError::Server(e) => match e {
                ClientError::Api {
                    code: Some(code), ..
                } => (*code).into(),
                ClientError::Api { status, .. } if *status < 500 => Self::BadRequest,
                ClientError::Api { .. } | ClientError::Header(_) | ClientError::Json(_) => {
                    Self::Internal
                }
                ClientError::Transport(_) => Self::Transport,
                ClientError::MissingPrivateKey => Self::Wallet,
            },
            SdkDriverError::Chain(_) => Self::Chain,
            SdkDriverError::Store(_) | SdkDriverError::RecordVersion(..) => Self::Store,
            SdkDriverError::Wallet(_) => Self::Wallet,
            SdkDriverError::Destination(_) => Self::Destination,
            SdkDriverError::Refused(_) => Self::Refused,
            SdkDriverError::Unsigned(_) => Self::Unsigned,
            SdkDriverError::BadSignature(_) => Self::BadSignature,
            SdkDriverError::ChainNotConfigured(_) => Self::ChainNotConfigured,
            SdkDriverError::Unknown(_) => Self::UnknownSwap,
            SdkDriverError::Unsupported(_) => Self::Unsupported,
            SdkDriverError::QuoteExpired => Self::QuoteExpired,
            SdkDriverError::WrongSwapType => Self::WrongSwapType,
            SdkDriverError::NotQuotable(_) => Self::NotQuotable,
            SdkDriverError::AmountOutOfRange { .. } => Self::AmountOutOfRange,
            SdkDriverError::Tx(_) | SdkDriverError::Utxo(_) | SdkDriverError::MissingField(..) => {
                Self::Internal
            }
        }
    }
}

/// A bad argument or an unconvertible value: never retryable.
pub(super) fn invalid(e: impl std::fmt::Display) -> DriverError {
    DriverError {
        message: e.to_string(),
        code: DriverErrorCode::Invalid,
        http_status: None,
        retryable: false,
    }
}
