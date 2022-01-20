use std::{collections::HashMap, io::Write, sync::Once};

static SETUP: Once = Once::new();
pub const CUSTOMER_CONFIG: &str = "TestCustomer.toml";
pub const MERCHANT_CONFIG: &str = "TestMerchant.toml";

/// Encode the customizable fields of the zeekoe customer Config struct for testing.
fn customer_test_config() -> String {
    let m = HashMap::from([
        ("database", "{ sqlite = \"customer-sandbox.db\" }"),
        ("trust_certificate", "\"localhost.crt\""),
        ("tezos_account", "{ alias = \"alice\" }"),
        ("tezos_uri", "\"http://localhost:20000\""),
        ("self_delay", "120"),
        ("confirmation_depth", "1"),
    ]);

    m.into_iter().fold("".to_string(), |acc, (key, value)| {
        format!("{}{} = {}\n", acc, key.to_string(), value.to_string())
    })
}

/// Encode the customizable fields of the zeekoe merchant Config struct for testing.
fn merchant_test_config() -> String {
    let tezos_config = customer_test_config()
        .replace("alice", "bob")
        .replace("customer", "merchant")
        .replace("trust_certificate = \"localhost.crt\"\n", "");

    // helper to write out the service for ipv4 and v6 localhost addresses
    let generate_service = |addr: &str| {
        HashMap::from([
            ("address", addr),
            ("private_key", "localhost.key"),
            ("certificate", "localhost.crt"),
        ])
        .into_iter()
        .fold("\n[[service]]".to_string(), |acc, (key, value)| {
            format!("{}\n{} = \"{}\"", acc, key, value)
        })
    };

    format!(
        "{}{}\n{}",
        tezos_config,
        generate_service("::1"),
        generate_service("127.0.0.1")
    )
}

/// Write out the configuration in `contents` to the file at `path`.
fn write_config_file(path: &str, contents: String) {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap_or_else(|_| panic!("Could not open config file: {}", path))
        .write_all(contents.as_bytes())
        .unwrap_or_else(|_| panic!("Failed to write to config file: {}", path));
}

pub fn setup() {
    SETUP.call_once(|| {
        // write config options for each party
        write_config_file(CUSTOMER_CONFIG, customer_test_config());
        write_config_file(MERCHANT_CONFIG, merchant_test_config());
    });
}
