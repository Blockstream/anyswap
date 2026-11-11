//! On-chain primitives and API types for AnySwap, shared by the server and the SDK.

pub mod api;
pub mod codec;
#[cfg(feature = "esplora")]
pub mod esplora;
pub mod fee;
pub mod invoice;
pub mod script;
pub mod transaction;
pub mod types;
pub mod utils;
pub mod wallet;

pub mod musig;

pub mod auth;

use std::sync::OnceLock;

use bitcoin::secp256k1::{All, Secp256k1};

/// The version of the wire protocol this build speaks: the crate's full version.
/// Builds of one major and minor version speak the same protocol, see
/// [`protocol_compatible`].
pub const PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether `version` speaks this build's protocol: it has the major and minor
/// version of [`PROTOCOL_VERSION`], since a patch release never changes the
/// wire. A version without a patch part does not.
pub fn protocol_compatible(version: &str) -> bool {
    let mut parts = version.splitn(3, '.');
    parts.next() == Some(env!("CARGO_PKG_VERSION_MAJOR"))
        && parts.next() == Some(env!("CARGO_PKG_VERSION_MINOR"))
        && parts.next().is_some_and(|patch| !patch.is_empty())
}

/// Shared secp256k1 context, lazily initialized once per process.
pub fn secp() -> &'static Secp256k1<All> {
    static SECP: OnceLock<Secp256k1<All>> = OnceLock::new();
    SECP.get_or_init(Secp256k1::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAJOR: &str = env!("CARGO_PKG_VERSION_MAJOR");
    const MINOR: &str = env!("CARGO_PKG_VERSION_MINOR");

    #[test]
    fn any_patch_of_the_minor_version_is_compatible() {
        assert!(protocol_compatible(PROTOCOL_VERSION));
        assert!(protocol_compatible(&format!("{MAJOR}.{MINOR}.0")));
        assert!(protocol_compatible(&format!("{MAJOR}.{MINOR}.99-rc.1")));
    }

    #[test]
    fn another_minor_or_a_version_without_a_patch_is_not() {
        let next_minor = MINOR.parse::<u64>().unwrap() + 1;
        assert!(!protocol_compatible(&format!("{MAJOR}.{next_minor}.0")));
        assert!(!protocol_compatible(&format!("{MAJOR}.{MINOR}")));
        assert!(!protocol_compatible(&format!("{MAJOR}.{MINOR}.")));
        assert!(!protocol_compatible(""));
    }
}
