use crate::amount::Currency;

#[derive(Debug, Clone, Copy)]
enum Bitcoin {}

#[derive(Debug, Clone, Copy)]
enum Zcash {}

#[derive(Debug, Clone, Copy)]
enum Tezos {}

impl Currency for Bitcoin {
    const MAXIMUM: u64 = 209_999_999_7690_000;
    const NAME: &'static str = "bitcoin";
    const SYMBOL: &'static str = "BTC";
    const UNIT_NAME: &'static str = "satoshi";
}

impl Currency for Zcash {
    const MAXIMUM: u64 = 209_999_999_7690_000;
    const NAME: &'static str = "zcash";
    const SYMBOL: &'static str = "ZEC";
    const UNIT_NAME: &'static str = "zatoshi";
}

impl Currency for Tezos {
    // Tezos has no maximum amount at present
    const NAME: &'static str = "tezos";
    const SYMBOL: &'static str = "XTZ";
    const UNIT_NAME: &'static str = "microtez";
}
