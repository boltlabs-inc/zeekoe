use generic_array::{ArrayLength, GenericArray};
use ring::rand::SecureRandom;

pub(crate) fn random_bytes<Length: ArrayLength<u8>>(
    rng: &dyn SecureRandom,
) -> GenericArray<u8, Length> {
    let mut vec = vec![0; Length::to_usize()];
    rng.fill(&mut vec)
        .expect("Error generating randomness in random_bytes()");
    GenericArray::from_exact_iter(vec).expect("Impossible length mismatch in random_bytes()")
}
