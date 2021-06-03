use {
    rusty_money::{crypto, Money, MoneyError},
    thiserror::Error,
};

pub type Amount = Money<'static, crypto::Currency>;

/// Parse an amount specified like "100.00 XTZ"
pub(crate) fn parse_amount(str: &str) -> Result<Amount, MoneyError> {
    if let Some((amount, currency)) = str.split_once(' ') {
        let currency = crypto::find(currency).ok_or(MoneyError::InvalidCurrency)?;
        Money::from_str(amount, currency)
    } else {
        Err(MoneyError::InvalidAmount)
    }
}

#[derive(Debug, Error)]
pub enum AmountParseError {
    #[error("Unknown currency: {0}")]
    UnknownCurrency(String),
    #[error("Invalid format for currency amount")]
    InvalidFormat,
}
