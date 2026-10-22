use thiserror::Error;

/// An exact signed quantity in the currency's smallest supported unit.
///
/// The wrapper deliberately leaves the currency scale to a separate policy
/// decision and exposes only checked arithmetic; callers cannot silently wrap
/// at the SQLite `INTEGER` boundary.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MinorUnits(i64);

impl MinorUnits {
    pub const ZERO: Self = Self(0);

    /// Creates an exact amount from a signed count of minor units.
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Returns the exact integer representation suitable for SQLite storage.
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Adds two exact amounts, returning an error instead of wrapping.
    pub fn checked_add(self, other: Self) -> Result<Self, MoneyError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(MoneyError::Overflow)
    }

    /// Subtracts two exact amounts, returning an error instead of wrapping.
    pub fn checked_sub(self, other: Self) -> Result<Self, MoneyError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or(MoneyError::Overflow)
    }

    /// Negates an exact amount, returning an error for `i64::MIN`.
    pub fn checked_neg(self) -> Result<Self, MoneyError> {
        self.0.checked_neg().map(Self).ok_or(MoneyError::Overflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MoneyError {
    #[error("minor-unit arithmetic overflowed SQLite's signed 64-bit integer range")]
    Overflow,
}
