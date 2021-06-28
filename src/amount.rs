use {
    rust_decimal::Decimal,
    rusty_money::{define_currency_set, FormattableCurrency, Money, MoneyError},
    std::{convert::TryInto, str::FromStr},
    thiserror::Error,
};

pub use supported::*;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct Amount {
    pub(crate) money: Money<'static, supported::Currency>,
}

impl FromStr for Amount {
    type Err = MoneyError;

    /// Parse an amount specified like "100.00 XTZ"
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((amount, currency)) = s.split_once(' ') {
            let currency = supported::find(currency).ok_or(MoneyError::InvalidCurrency)?;
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
    /// Convert this [`Amount`] into a unitless signed amount of the smallest denomination of its
    /// currency, or fail if it is not representable as such.
    pub fn try_into_minor_units(&self) -> Option<i64> {
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

    /// Get the currency of this [`Amount`].
    pub fn currency(&self) -> &'static supported::Currency {
        self.money.currency()
    }

    /// Convert a unitless signed number into an [`Amount`] in the given currency equal to that
    /// number of the smallest denomination of the currency.
    ///
    /// For example, one cent is the smallest denomination of the USD, so this function would
    /// interpret the number `1` as "0.01 USD", if the currency was USD.
    pub fn from_minor_units_of_currency(
        minor_units: i64,
        currency: &'static supported::Currency,
    ) -> Self {
        let minor_units: Decimal = minor_units.into();
        let major_units = minor_units / Decimal::from(10u32.pow(currency.exponent()));
        Self {
            money: Money::from_decimal(major_units, currency),
        }
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

// Define only the currencies supported by this application
define_currency_set!(
    supported {
        // Copied from the rusty_money crypto-currency definitions
        XTZ: {
            code: "XTZ",
            exponent: 6,
            locale: EnUs,
            minor_units: 1_000_000,
            name: "Tezos",
            symbol: "XTZ",
            symbol_first: false,
        }
    }
);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parse_and_extract_tezos() {
        let tezos_amount = Amount::from_str("12.34 XTZ").expect("failed to parse");
        let minor_amount = tezos_amount
            .try_into_minor_units()
            .expect("failed to get minor amount");
        assert_eq!(12_340_000, minor_amount);
    }

    #[test]
    fn round_trip_minor_units_tezos() {
        let microtez = Amount::from_minor_units_of_currency(1, XTZ);
        assert_eq!(1, microtez.try_into_minor_units().unwrap());
    }
}
