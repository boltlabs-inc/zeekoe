use generic_array::GenericArray;
use ring::rand::SecureRandom;
use sha2::Digest;

use crate::chain::Arbiter;
use crate::util::*;

#[derive(Debug, Clone)]
pub struct RevocationLock<J: Arbiter>(GenericArray<u8, <J::RevocationHash as Digest>::OutputSize>);

impl<J: Arbiter> RevocationLock<J> {
    /// Get the underlying bytes of a revocation lock.
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// Check that a revocation lock corresponds to a given revocation secret.
    pub fn matches_secret(&self, secret: &RevocationSecret<J>) -> bool {
        J::RevocationHash::digest(&secret.0) == self.0
    }
}

/// A revocation secret is a randomly generated token used to prove that the previously-released
/// revocation lock corresponded to it.
#[derive(Debug, Clone)]
pub struct RevocationSecret<J: Arbiter>(GenericArray<u8, J::RevocationSecurityParameter>);

impl<J: Arbiter> RevocationSecret<J> {
    /// Get the underlying bytes of a revocation secret.
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// A `Revocation` is a pair of a *revocation lock* and a *revocation secret*.
/// The revocation lock is a *hiding commitment* to the secret; that is, if the
/// secret is revealed, one can verify that it corresponded to the lock.
/// However, knowledge of the lock does not allow someone to deduce the secret.
/// To create a random new revocation pair, use the `new` function.
pub struct Revocation<J: Arbiter> {
    lock: RevocationLock<J>,
    secret: RevocationSecret<J>,
}

impl<J: Arbiter> Revocation<J> {
    /// Create a new random revocation lock/secret pair, according to the
    /// security parameter and hash function for this arbiter.
    pub fn new(rng: &dyn SecureRandom) -> Revocation<J> {
        // Generate a random secret of the appropriate length (inferred from the revocation security
        // parameter defined in `J`)
        let secret = random_bytes(rng);

        // Take its hash to compute the revocation lock
        let lock = J::RevocationHash::digest(secret.as_slice());
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
