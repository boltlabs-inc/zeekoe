use generic_array::{ArrayLength, GenericArray};
use ring::rand::SecureRandom;
use sha2::Digest;

use crate::chain::Arbiter;

#[derive(Debug, Clone)]
pub struct RevocationLock<J: Arbiter>(GenericArray<u8, <J::RevocationHash as Digest>::OutputSize>);

impl<J: Arbiter> RevocationLock<J> {
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

#[derive(Debug, Clone)]
pub struct RevocationSecret<SecurityParameter: ArrayLength<u8>>(
    GenericArray<u8, SecurityParameter>,
);

impl<SecurityParameter: ArrayLength<u8>> RevocationSecret<SecurityParameter> {
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub struct Revocation<J: Arbiter> {
    lock: RevocationLock<J>,
    secret: RevocationSecret<J::RevocationSecurityParameter>,
}

impl<J: Arbiter> Revocation<J>
where
    <J::RevocationSecurityParameter as ArrayLength<u8>>::ArrayType:
        ring::rand::RandomlyConstructable,
    J::RevocationSecurityParameter: ArrayLength<u8, ArrayType = [u8]>,
{
    /// Create a new random revocation lock/secret pair, according to the
    /// security parameter and hash function for this arbiter.
    pub fn new(rng: &dyn SecureRandom) -> Revocation<J> {
        let secret: J::RevocationSecurityParameter::ArrayType = ring::rand::generate(rng)
            .expect("Random revocation secret generation failed")
            .expose();
        let secret: GenericArray<u8, J::RevocationSecurityParameter> =
            GenericArray::from_slice(&secret).clone();
        let lock: GenericArray<u8, J::RevocationHash::OutputSize> =
            J::RevocationHash::digest(&secret);
        Revocation {
            lock: RevocationLock(lock),
            secret: RevocationSecret(secret),
        }
    }

    /// Get a reference to the revocation lock. This method may be called many
    /// times, unlike [`Revocation::secret`], which consumes `self`.
    pub fn lock(&self) -> &RevocationLock<J> {
        &self.lock
    }

    /// Reveal the revocation secret. This method consumes `self` to ensure that
    /// the caller cannot re-use the revocation lock after revealing the secret.
    pub fn secret(self) -> RevocationSecret<J> {
        self.secret
    }
}
