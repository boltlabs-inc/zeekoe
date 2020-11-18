use crate::{customer, wire::establish};
use ring::rand::SecureRandom;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::{Add, Mul, Sub};

use crate::amount::{Amount, Currency};
use crate::revocation::RevocationLock;

pub struct InvalidClosingAuthorization;

pub trait Arbiter {
    /// The channel identifier for this payment network.
    type ChannelId: Debug + Clone;

    /// The hash function used for revocation locks on this payment network.
    type RevocationHash: digest::Digest;

    /// The security parameter used for revocation locks on this payment network.
    type RevocationSecurityParameter: generic_array::ArrayLength<u8>;

    /// The security parameter used for the signature scheme.
    type SignatureSchemeSecurityParameter: generic_array::ArrayLength<u8>;

    /// The currency used on this payment network.
    type TransactionCurrency: Currency;

    /// The signature scheme used for signing transactions on this network.
    type TransactionSignatureScheme: SignatureScheme<Self::SignatureSchemeSecurityParameter, [u8]>;
}

pub trait SignatureScheme<SecurityParameter: generic_array::ArrayLength<u8>, T>
where
    T: ?Sized,
{
    /// The public key for the signature scheme.
    type PublicKey: Clone + Into<Vec<u8>>;

    /// The private key for the signature scheme.
    type PrivateKey: Clone;

    /// The type of a signature.
    type Signature;

    /// Generate a new fresh pair of private and public keys.
    fn key_pair(rng: &dyn SecureRandom) -> (Self::PublicKey, Self::PrivateKey);

    /// Sign some data and produce a signature for it which can be verified later.
    fn sign(private_key: &Self::PrivateKey, rng: &dyn SecureRandom, data: &T) -> Self::Signature;

    /// Check that a signature matchers some given data.
    fn verify(public_key: &Self::PublicKey, signature: &Self::Signature, data: &T) -> bool;
}
