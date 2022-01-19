use std::{collections::HashMap, io::Write, sync::Once};

static SETUP: Once = Once::new();
const CUSTOMER_CONFIG: &str = "TestCustomer.toml";

/// Customizable fields of the zeekoe customer Config struct
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

pub fn setup() {
    SETUP.call_once(|| {
        // write customer config options to a file
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(CUSTOMER_CONFIG)
            .expect("Could not open file.")
            .write_all(customer_test_config().as_bytes())
            .expect("Failed to write customer config");
    });
}
