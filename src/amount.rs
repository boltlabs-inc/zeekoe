use std::convert::TryFrom;
use std::fmt::Debug;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::{Add, Mul, Sub};

/// A trait for defining currencies. There is no need to have a value-level object corresponding to
/// a `Currency`, so it's intended that this trait be instantiated on an empty `enum`.
pub trait Currency
where
    Self: Sized + Copy,
{
    /// The largest representable amount of currency. By default, this is equal to the maximum value
    /// of `u64`, but for other currencies with different maxima (i.e. most cryptocurrencies), this
    /// should be changed.
    const MAXIMUM: u64 = u64::max_value();

    /// The printable name of the currency in lower-case singular form, e.g. "dollar" or "bitcoin".
    const NAME: &'static str;

    /// The symbol for the currency, e.g. "USD" or "BTC".
    const SYMBOL: &'static str;

    /// The name of the atomic currency unit in lower-case singular form, e.g. "cent" or "satoshi".
    const UNIT_NAME: &'static str;

    /// How to format an amount of the currency. By default, this has the same behavior as
    /// `fmt_as_units`.
    fn fmt_amount(amount: &Amount<Self>, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        amount.fmt_as_units(f)
    }
}

impl<C: Currency> Display for Amount<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        <C as Currency>::fmt_amount(self, f)
    }
}

/// An amount of some currency.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Copy)]
pub struct Amount<C> {
    /// The actual number of atomic units of currency.
    units: u64,
    /// The currency in question (represented at the type level).
    currency: PhantomData<C>,
}

impl<C: Currency> Amount<C> {
    /// This renders the currency amount as something like "100 cents" or "1 satoshi" (handling
    /// pluralization automatically).
    fn fmt_as_units(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{} {}{}",
            self.units,
            C::UNIT_NAME,
            if self.units > 1 { "s" } else { "" }
        )
    }
}

impl<C> std::default::Default for Amount<C> {
    fn default() -> Amount<C> {
        Amount {
            units: 0,
            currency: PhantomData,
        }
    }
}

impl<C: Currency> Add for Amount<C> {
    type Output = Result<Amount<C>, UnrepresentableCurrencyAmount<C>>;

    fn add(self, other: Amount<C>) -> Self::Output {
        let sum = match self.units.checked_add(other.units) {
            None => Err(UnrepresentableCurrencyAmount::Overflow)?,
            Some(sum) => sum,
        };
        if sum <= C::MAXIMUM {
            Ok(Amount {
                units: sum,
                currency: PhantomData,
            })
        } else {
            Err(UnrepresentableCurrencyAmount::AboveMaximum {
                units: sum,
                currency: PhantomData,
            })
        }
    }
}

impl<C: Currency> Sub for Amount<C> {
    type Output = Option<Amount<C>>;

    fn sub(self, other: Amount<C>) -> Option<Amount<C>> {
        let diff = self.units.checked_sub(other.units)?;
        Some(Amount {
            units: diff,
            currency: PhantomData,
        })
    }
}

#[derive(Clone, Copy)]
/// An error indicating that an amount of currency was not representable in a given currency.
pub enum UnrepresentableCurrencyAmount<C> {
    AboveMaximum {
        units: u64,
        currency: PhantomData<C>,
    },
    Overflow,
    Underflow,
}

impl<C> std::fmt::Debug for UnrepresentableCurrencyAmount<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UnrepresentableCurrencyAmount::AboveMaximum { units, .. } => f
                .debug_struct("UnrepresentableCurrencyAmount::AboveMaximum")
                .field("amount", &units)
                .field("currency", &PhantomData::<C>)
                .finish(),
            UnrepresentableCurrencyAmount::Overflow => f
                .debug_struct("UnrepresentableCurrencyAmount::Overflow")
                .finish(),
            UnrepresentableCurrencyAmount::Underflow => f
                .debug_struct("UnrepresentableCurrencyAmount::Underflow")
                .finish(),
        }
    }
}

impl<C: Currency> std::fmt::Display for UnrepresentableCurrencyAmount<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UnrepresentableCurrencyAmount::AboveMaximum { units, .. } => {
                write!(
                f,
                "Currency amount {} is higher than the maximum representable amount for {}s ({})",
                Amount { units: *units, currency: PhantomData::<C> },
                C::NAME,
                Amount { units: C::MAXIMUM, currency: PhantomData::<C> },
            )
            }
            UnrepresentableCurrencyAmount::Overflow => {
                write!(f, "Currency overflow: beyond bounds of u64")
            }
            UnrepresentableCurrencyAmount::Underflow => write!(f, "Currency underflow: below zero"),
        }
    }
}

impl<C: Currency> std::error::Error for UnrepresentableCurrencyAmount<C> where Amount<C>: Display {}

impl<C: Currency> TryFrom<u64> for Amount<C>
where
    Amount<C>: Display,
{
    type Error = UnrepresentableCurrencyAmount<C>;

    fn try_from(units: u64) -> Result<Amount<C>, UnrepresentableCurrencyAmount<C>> {
        if units <= C::MAXIMUM {
            Ok(Amount {
                units,
                currency: PhantomData,
            })
        } else {
            Err(UnrepresentableCurrencyAmount::AboveMaximum {
                units,
                currency: PhantomData,
            })
        }
    }
}

impl<C: Currency> Into<u64> for Amount<C>
where
    Amount<C>: Display,
{
    fn into(self) -> u64 {
        self.units
    }
}
