use {
    rust_decimal::Decimal,
    rusty_money::{crypto, FormattableCurrency, Money, MoneyError},
    std::{convert::TryInto, str::FromStr},
    thiserror::Error,
};

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Amount {
    pub(crate) money: Money<'static, crypto::Currency>,
}

impl FromStr for Amount {
    type Err = MoneyError;

    /// Parse an amount specified like "100.00 XTZ"
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((amount, currency)) = s.split_once(' ') {
            let currency = crypto::find(currency).ok_or(MoneyError::InvalidCurrency)?;
            let money = Money::from_str(amount, currency)?;
            if money.is_positive() {
                Ok(Amount { money })
            } else {
                Err(MoneyError::InvalidAmount)
            }
        } else {
            Err(MoneyError::InvalidAmount)
        }
    }
}

impl Amount {
    pub fn as_minor_units(&self) -> Option<i64> {
        // The amount of money, as a `Decimal` in *major* units (e.g. 1 USD = 1.00)
        let amount: &Decimal = self.money.amount();

        // The number of decimal places used to represent *minor* units (e.g. for USD, this is 2)
        let exponent: u32 = self.money.currency().exponent();

        // The number of minor units equivalent to the amount, as a `Decimal` (multiply by 10^e)
        let minor_units = amount.checked_mul(Decimal::from(10u32.checked_pow(exponent)?))?;

        // If the amount of currency has a fractional amount of minor units, fail
        if minor_units != minor_units.trunc() {
            return None;
        }

        // Convert whole-numbered `Decimal` of minor units into an `i64`
        let scale: u32 = minor_units.scale();
        let mantissa: i64 = minor_units.mantissa().try_into().ok()?;
        let minor_units: i64 = mantissa.checked_div(10i64.checked_pow(scale)?)?;

        Some(minor_units)
    }
}

#[allow(unused)]
#[derive(Debug, Error)]
pub enum AmountParseError {
    #[error("Unknown currency: {0}")]
    UnknownCurrency(String),
    #[error("Invalid format for currency amount")]
    InvalidFormat,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_and_extract_tezos() {
        let tezos_amount = Amount::from_str("12.34 XTZ").expect("failed to parse");
        let minor_amount = tezos_amount
            .as_minor_units()
            .expect("failed to get minor amount");
        assert_eq!(12_340_000, minor_amount);
    }
}
