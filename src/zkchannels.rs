//! The complete zkchannels protocol implementation
pub mod customer;
pub mod merchant;

use rand::{rngs::StdRng, SeedableRng};

/// Get a pseudorandom number generator. This should only be used in two circumstances:
/// (1) you need an RNG and you don't have one
/// (2) in order to use your RNG, you'd have to clone it (e.g. spawning a new thread with
/// an `async move` block)
///
/// Don't clone RNGs!
pub fn zkchannels_rng() -> StdRng {
    StdRng::from_entropy()
}
