use crate::{customer, wire::establish};
use ring::rand::SecureRandom;
use std::convert::TryFrom;
use std::fmt::Debug;

use crate::revocation::RevocationLock;

pub struct InvalidClosingAuthorization;

pub struct InvalidCurrencyAmount(u64);

/// The set of all types and operations necessary to define a backend blockchain for a zkchannels
/// instantiation.
pub trait Chain
where
    Self::ChannelId: Debug + Clone,
    Self::ChannelPublicKey: Clone + Into<Vec<u8>>,
    Self::ChannelPrivateKey: Clone + Into<Vec<u8>>,
    Self::ClosingAuthorization: Clone + TryFrom<Vec<u8>, Error = InvalidClosingAuthorization>,
    Self::EscrowAuthorization: Clone + Into<Vec<u8>>,
    Self::ExpiryAuthorization: Clone + Into<Vec<u8>>,
    Self::Currency: Clone + Into<u64> + TryFrom<u64, Error = InvalidCurrencyAmount>,
{
    /// The channel identifier for this blockchain backend.
    type ChannelId;
    /// The type for on-chain public keys used on this blockchain.
    type ChannelPublicKey;
    /// The type for on-chain private keys used on this blockchain.
    type ChannelPrivateKey;
    /// The type for revocation locks used on this blockchain.
    type ClosingAuthorization;
    /// The type of escrow authorization signatures on this blockchain.
    type EscrowAuthorization;
    /// The type of expiry authorization signatures (if any) on this blockchain.
    type ExpiryAuthorization;
    /// The type of currency on this blockchain.
    type Currency;

    /// The maximum representable amount of money on the blockchain.
    const MAX_CURRENCY: Self::Currency;

    /// A zero amount of money on the blockchain.
    const ZERO_CURRENCY: Self::Currency;

    /// Generate a fresh random channel keypair for this blockchain.
    fn channel_keypair(rng: &dyn SecureRandom)
        -> (Self::ChannelPublicKey, Self::ChannelPrivateKey);

    fn customer_escrow(
        &mut self,
        // TODO: fill in the rest of the parameters here
        establish: customer::channel::StreamingMethod<establish::Request, establish::Reply>,
    ) -> Result<(), customer::channel::Error>;
}

pub trait Arbiter {
    /// The channel identifier for this payment network.
    type ChannelId: Debug + Clone;

    /// The hash function used for revocation locks on this payment network.
    type RevocationHash: digest::Digest;

    /// The security parameter used for revocation locks on this payment network.
    type RevocationSecurityParameter: generic_array::ArrayLength<u8>;
}
