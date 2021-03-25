use num::traits::{CheckedAdd, CheckedSub, One};
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt::Debug;
use std::fmt::Display;
use std::marker::PhantomData;
use std::ops::{Add, Sub};

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

    /// How to format an amount of the currency. By default, this renders the currency amount as
    /// something like "100 cents" or "1 satoshi" (handling pluralization automatically).
    fn fmt_amount(amount: &Amount<Self, u64>, f: &mut std::fmt::Formatter) -> std::fmt::Result {
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
pub struct Amount<C, T = u64> {
    /// The actual number of atomic units of currency.
    units: T,
    /// The currency in question (represented at the type level).
    currency: PhantomData<C>,
}

impl<C: Currency, T: One + Eq + Display> Amount<C, T> {
    /// This renders the currency amount as something like "100 cents" or "1 satoshi" (handling
    /// pluralization automatically).
    fn fmt_as_units(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "{} {}{}",
            self.units,
            C::UNIT_NAME,
            if self.units != T::one() { "s" } else { "" }
        )
    }

    /// Cast the underlying storage type of an `Amount` so that operations (e.g. addition) will use
    /// the new storage type. To keep the user from going up a creek without a paddle, this is
    /// restricted to those types which can at least *try* to be cast back to a `u64`, the default
    /// backing storage of an `Amount`.
    pub fn cast<S: From<T> + TryInto<u64>>(self) -> Amount<C, S> {
        Amount {
            units: self.units.into(),
            currency: PhantomData,
        }
    }
}

impl<C, T: std::default::Default> std::default::Default for Amount<C, T> {
    fn default() -> Amount<C, T> {
        Amount {
            units: T::default(),
            currency: PhantomData,
        }
    }
}

impl<C: Currency, T: CheckedAdd + Ord + From<u64> + TryInto<u64>> Add for Amount<C, T> {
    type Output = Result<Amount<C, T>, CurrencyError<C>>;

    fn add(self, other: Amount<C, T>) -> Self::Output {
        let sum = match self.units.checked_add(&other.units) {
            None => return Err(CurrencyError::Overflow),
            Some(sum) => sum,
        };
        if sum <= C::MAXIMUM.into() {
            Ok(Amount {
                units: sum,
                currency: PhantomData,
            })
        } else {
            Err(match sum.try_into() {
                Ok(sum) => CurrencyError::AboveMaximum {
                    units: sum,
                    currency: PhantomData,
                },
                Err(_) => CurrencyError::Overflow,
            })
        }
    }
}

impl<C: Currency, T: CheckedSub> Sub for Amount<C, T> {
    type Output = Result<Amount<C, T>, CurrencyError<C>>;

    fn sub(self, other: Amount<C, T>) -> Result<Amount<C, T>, CurrencyError<C>> {
        let diff = match self.units.checked_sub(&other.units) {
            None => return Err(CurrencyError::Underflow),
            Some(diff) => diff,
        };
        Ok(Amount {
            units: diff,
            currency: PhantomData,
        })
    }
}
#[derive(Clone, Copy)]
/// An error indicating that an amount of currency was not representable in a given currency.
pub enum CurrencyError<C> {
    AboveMaximum {
        units: u64,
        currency: PhantomData<C>,
    },
    Overflow,
    Underflow,
}

impl<C> std::fmt::Debug for CurrencyError<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CurrencyError::AboveMaximum { units, .. } => f
                .debug_struct("CurrencyError::AboveMaximum")
                .field("amount", &units)
                .field("currency", &PhantomData::<C>)
                .finish(),
            CurrencyError::Overflow => f.debug_struct("CurrencyError::Overflow").finish(),
            CurrencyError::Underflow => f.debug_struct("CurrencyError::Underflow").finish(),
        }
    }
}

impl<C: Currency> std::fmt::Display for CurrencyError<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CurrencyError::AboveMaximum { units, .. } => {
                write!(
                f,
                "Currency amount {} is higher than the maximum representable amount for {}s ({})",
                Amount { units: *units, currency: PhantomData::<C> },
                C::NAME,
                Amount { units: C::MAXIMUM, currency: PhantomData::<C> },
            )
            }
            CurrencyError::Overflow => {
                write!(f, "Currency overflow: beyond bounds of u64")
            }
            CurrencyError::Underflow => write!(f, "Currency underflow: below zero"),
        }
    }
}

impl<C: Currency> std::error::Error for CurrencyError<C> {}

impl<C: Currency> TryFrom<u64> for Amount<C, u64> {
    type Error = CurrencyError<C>;

    fn try_from(units: u64) -> Result<Amount<C>, CurrencyError<C>> {
        if units <= C::MAXIMUM {
            Ok(Amount {
                units,
                currency: PhantomData,
            })
        } else {
            Err(CurrencyError::AboveMaximum {
                units,
                currency: PhantomData,
            })
        }
    }
}

impl<C: Currency, T: Into<u64>> From<Amount<C, T>> for u64 {
    fn from(amount: Amount<C, T>) -> u64 {
        amount.units.into()
    }
}
