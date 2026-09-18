// SPDX-License-Identifier: AGPL-3.0-or-later
//! Money: an exact decimal amount, carried as a scaled integer with no floating
//! point. A cost that has passed through a binary float and come back as
//! 1234.5600000000001 cannot be reconciled against a contract; when the subject
//! is money, arithmetic that is merely close is a defect. So a money amount is a
//! pair of integers - minor units and a scale - and every operation on it is
//! exact: 0.1 + 0.2 is 0.3, never 0.30000000000000004.
//!
//! The stored form is canonical: one amount has exactly one representation, so
//! equality and ordering compare values, not the accidental number of decimal
//! places a spreadsheet wrote. The fewest decimal places that express a value
//! are kept (12.50 is 12.5, 12 is 12, -3.25 is -3.25).

use std::cmp::Ordering;
use std::fmt;
use std::ops::Add;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The most fractional digits a money amount may carry. Real currencies use at
/// most a handful (two is the norm, three for a few); nine is generous headroom.
/// The bound is what keeps addition exact and safe: adding aligns both operands
/// to the finer scale, and a bounded scale keeps that alignment inside i128.
const MAX_SCALE: u32 = 9;

/// The largest absolute amount a money value may carry, in whole units of the
/// currency (one quintillion). No ERP export, contract schedule or spreadsheet
/// holds a cost this large; the bound exists so that aligning two values to a
/// common scale and summing them can never overflow i128. A number above it is
/// not a cost, and parse refuses it rather than pretending it is money.
const MAX_VALUE: i128 = 1_000_000_000_000_000_000;

/// The error returned when a string is not a valid decimal money amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoneyParseError;

impl fmt::Display for MoneyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a valid decimal money amount")
    }
}

impl std::error::Error for MoneyParseError {}

/// An exact decimal amount: value = minor_units * 10^-scale, with no floating
/// point anywhere in the representation or the arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Money {
    /// The amount in minor units (cents when scale is 2). Canonical: when scale
    /// is nonzero, minor_units has no trailing zero.
    minor_units: i128,
    /// The number of decimal places.
    scale: u32,
}

impl Money {
    /// The amount in minor units: minor_units * 10^-scale is the value.
    pub fn minor_units(&self) -> i128 {
        self.minor_units
    }

    /// The number of decimal places the stored minor units are scaled by.
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// Build an amount from minor units and a scale, reducing it to its
    /// canonical form. This is the low-level constructor and does not enforce
    /// the parse bounds; Money::parse is the validated entry point.
    pub fn from_minor_units(minor_units: i128, scale: u32) -> Self {
        Self::reduce(minor_units, scale)
    }

    /// Parse a decimal string: an optional sign, an integer part, and an
    /// optional decimal point followed by a fractional part. Leading and
    /// trailing whitespace is ignored. Currency symbols, thousands separators
    /// and any other character are not money and are refused. A whole number
    /// parses at scale zero, a decimal at the scale it states, and a negative
    /// amount (a credit) under the same rules.
    pub fn parse(input: &str) -> Result<Self, MoneyParseError> {
        let text = input.trim();
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return Err(MoneyParseError);
        }

        let mut index = 0;
        let mut negative = false;
        match bytes[0] {
            b'+' => index = 1,
            b'-' => {
                negative = true;
                index = 1;
            }
            _ => {}
        }

        let mut minor: i128 = 0;
        let mut scale: u32 = 0;
        let mut digits = 0usize;

        // Integer part.
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            minor = minor
                .checked_mul(10)
                .and_then(|m| m.checked_add((bytes[index] - b'0') as i128))
                .ok_or(MoneyParseError)?;
            digits += 1;
            index += 1;
        }

        // Fractional part.
        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            let fractional_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                minor = minor
                    .checked_mul(10)
                    .and_then(|m| m.checked_add((bytes[index] - b'0') as i128))
                    .ok_or(MoneyParseError)?;
                scale += 1;
                digits += 1;
                index += 1;
            }
            if index == fractional_start {
                // A point with nothing after it ("12." or ".") is not an amount.
                return Err(MoneyParseError);
            }
        }

        // A sign with no digits, or trailing characters, is not money.
        if digits == 0 || index != bytes.len() {
            return Err(MoneyParseError);
        }

        if negative {
            minor = -minor;
        }

        let value = Self::reduce(minor, scale);

        // Bound the number of decimal places and the magnitude so that aligning
        // two values to a common scale and summing them can never overflow i128.
        if value.scale > MAX_SCALE {
            return Err(MoneyParseError);
        }
        if value.minor_units.abs() > MAX_VALUE * 10i128.pow(value.scale) {
            return Err(MoneyParseError);
        }
        Ok(value)
    }

    /// Reduce to the canonical form by dropping trailing zeros, so that one
    /// amount has exactly one representation and equality is value equality.
    fn reduce(mut minor_units: i128, mut scale: u32) -> Self {
        while scale > 0 && minor_units % 10 == 0 {
            minor_units /= 10;
            scale -= 1;
        }
        Self { minor_units, scale }
    }
}

impl Add for Money {
    type Output = Money;

    fn add(self, other: Money) -> Money {
        // Align to the finer scale, then add the aligned minor units. With the
        // parse bounds the aligned operands stay far inside i128, so this is
        // exact and needs no rounding and no floating point.
        let common = self.scale.max(other.scale);
        let left = self.minor_units * 10i128.pow(common - self.scale);
        let right = other.minor_units * 10i128.pow(common - other.scale);
        Money::reduce(left + right, common)
    }
}

impl Ord for Money {
    fn cmp(&self, other: &Self) -> Ordering {
        // Compare values by aligning both to the finer scale: value ordering,
        // not a comparison of the two stored encodings.
        let common = self.scale.max(other.scale);
        let left = self.minor_units * 10i128.pow(common - self.scale);
        let right = other.minor_units * 10i128.pow(common - other.scale);
        left.cmp(&right)
    }
}

impl PartialOrd for Money {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            return write!(f, "{}", self.minor_units);
        }
        let negative = self.minor_units < 0;
        let magnitude = self.minor_units.unsigned_abs();
        let divisor = 10u128.pow(self.scale);
        let whole = magnitude / divisor;
        let fraction = magnitude % divisor;
        if negative {
            write!(f, "-")?;
        }
        write!(
            f,
            "{}.{:0width$}",
            whole,
            fraction,
            width = self.scale as usize
        )
    }
}

impl FromStr for Money {
    type Err = MoneyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Money::parse(s)
    }
}

impl From<Money> for String {
    fn from(value: Money) -> Self {
        value.to_string()
    }
}

impl TryFrom<String> for Money {
    type Error = MoneyParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Money::parse(&value)
    }
}
