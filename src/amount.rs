use {
    rust_decimal::Decimal,
    rusty_money::{define_currency_set, FormattableCurrency, Money, MoneyError},
    std::{
        convert::TryInto,
        fmt::{self, Display},
        num::TryFromIntError,
        str::FromStr,
    },
    thiserror::Error,
};

pub use supported::*;
use zkabacus_crypto::{
    CustomerBalance, Error as PaymentAmountError, MerchantBalance, PaymentAmount,
};

use crate::protocol::Party;

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

impl From<CustomerBalance> for Amount {
    fn from(value: CustomerBalance) -> Self {
        // This unwrap is safe because the `into_inner` function guarantees a u64
        // with value < i64::Max
        // Developer note: to extend to multiple currencies, we will have to do something more
        // clever than hardcoding XTZ here.
        Amount::from_minor_units_of_currency(value.into_inner().try_into().unwrap(), XTZ)
    }
}

impl From<MerchantBalance> for Amount {
    fn from(value: MerchantBalance) -> Self {
        // This unwrap is safe because the `into_inner` function guarantees a u64
        // with value < i64::Max
        // Developer note: to extend to multiple currencies, we will have to do something more
        // clever than hardcoding XTZ here.
        Amount::from_minor_units_of_currency(value.into_inner().try_into().unwrap(), XTZ)
    }
}

impl TryInto<PaymentAmount> for Amount {
    type Error = AmountParseError;

    fn try_into(self) -> Result<PaymentAmount, Self::Error> {
        // Convert the payment amount appropriately
        let minor_units: i64 = self
            .try_into_minor_units()
            .ok_or(AmountParseError::InvalidValue)?;

        // Squash into PaymentAmount
        Ok(if minor_units < 0 {
            PaymentAmount::pay_customer(minor_units.abs() as u64)
        } else {
            PaymentAmount::pay_merchant(minor_units as u64)
        }?)
    }
}

macro_rules! try_into_balance {
    ($balance_type:ident, $party:ident) => {
        impl TryInto<$balance_type> for Amount {
            type Error = BalanceConversionError;

            fn try_into(self) -> Result<$balance_type, Self::Error> {
                $balance_type::try_new(
                    self.try_into_minor_units()
                        .ok_or_else(|| Self::Error::InvalidDeposit(Party::Customer))?
                        .try_into()?,
                )
                .map_err(|_| Self::Error::InvalidDeposit(Party::$party))
            }
        }
    };
}

try_into_balance!(CustomerBalance, Customer);
try_into_balance!(MerchantBalance, Merchant);

#[derive(Debug, Error)]
pub enum BalanceConversionError {
    #[error("Could not convert {0} deposit into a valid balance")]
    InvalidDeposit(Party),
    #[error(transparent)]
    BalanceTooLarge(#[from] TryFromIntError),
}

impl Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.money.amount().fmt(f)?;
        write!(f, " ")?;
        self.money.currency().fmt(f)
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

#[derive(Debug, Error)]
pub enum AmountParseError {
    #[error("Unknown currency: {0}")]
    UnknownCurrency(String),
    #[error("Invalid format for currency amount")]
    InvalidFormat,
    #[error("Payment amount invalid for currency or out of range for channel")]
    InvalidValue,
    #[error(transparent)]
    InvalidPaymentAmount(#[from] PaymentAmountError),
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

    #[test]
    fn test_balance_parsing() {
        // Parsing fails with too many decimal places
        let too_many_decimals_amount = Amount::from_str("1.55555555 XTZ").unwrap();

        let customer_balance: Result<CustomerBalance, _> =
            too_many_decimals_amount.clone().try_into();
        assert!(customer_balance.is_err());

        let merchant_balance: Result<MerchantBalance, _> = too_many_decimals_amount.try_into();
        assert!(merchant_balance.is_err());

        // Pasring fails on too-large numbers
        let bad_amount = Amount::from_str("9223372036854775810 XTZ");
        assert!(
            bad_amount.is_err()
                || TryInto::<CustomerBalance>::try_into(bad_amount.unwrap()).is_err()
        );

        let bad_amount = Amount::from_str("9223372036854775810 XTZ");
        assert!(
            bad_amount.is_err()
                || TryInto::<MerchantBalance>::try_into(bad_amount.unwrap()).is_err()
        );
    }
}
