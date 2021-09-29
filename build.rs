use canonicalize_json_micheline::{canonicalize_json_micheline, CanonicalizeError};

const CANONICAL_CONTRACT_PATH: &str = "src/escrow/zkchannels_contract_canonical.json";

fn main() -> Result<(), CanonicalizeError> {
    println!("cargo:rerun-if-changed=src/database/migrations/merchant");
    println!("cargo:rerun-if-changed=src/escrow/zkchannels_contract.json");

    let contract_json = include_str!("src/escrow/zkchannels_contract.json");
    let canonical_contract_json = canonicalize_json_micheline(contract_json)?;
    std::fs::write(CANONICAL_CONTRACT_PATH, canonical_contract_json).unwrap_or_else(|_| {
        panic!(
            "Unable to write canonicalized contract to {}",
            CANONICAL_CONTRACT_PATH
        )
    });

    Ok(())
}
