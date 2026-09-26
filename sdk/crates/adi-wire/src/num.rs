//! Exact decimal numbers, as ADI-P 10.2.2 puts them on the wire.
//!
//! The engine reasons in `f64` and the appliance stores `{ mantissa, exponent }`
//! integers. This is where the two meet, and it is the one conversion in the
//! system that must not lose anything: a range check against a safety limit has
//! to be exact rather than nearly right, which is why the wire carries a
//! decimal fraction and refuses binary floats.

use adi_protocol::units::{Quantity, Unit};

use crate::WireError;

/// The mirror of `adip_num_t`: a value as `mantissa * 10^exponent`, with its
/// unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Num {
    pub mantissa: i32,
    pub exponent: i8,
    pub unit: Unit,
}

/// How far below the decimal point a conversion will look.
///
/// Ten decimal places is far past any physical quantity this protocol carries,
/// and stopping somewhere is what turns "this value cannot be represented" into
/// an error rather than a hang.
const MIN_EXPONENT: i8 = -9;

/// How far the reconstructed value may sit from the original.
///
/// The comparison is deliberately made after rebuilding `mantissa * 10^exp`
/// rather than on the scaled value, and this is not a detail. A tolerance
/// applied to the scaled number grows with the exponent: at nine decimal
/// places the scaled form of a third is around 3.3e8, where a relative 1e-9
/// is 0.33, loose enough to accept a third as exact. Checking what comes back
/// out keeps the tolerance in the units the value is actually in.
const TOLERANCE: f64 = 1e-9;

impl Num {
    pub fn new(mantissa: i32, exponent: i8, unit: Unit) -> Self {
        Self { mantissa, exponent, unit }
    }

    /// Convert a quantity, keeping its value exactly.
    ///
    /// Finds the largest exponent, meaning the fewest decimal places, that
    /// represents the value as a whole mantissa. 36.5 becomes 365 at -1 rather
    /// than 3650 at -2, because the shorter form is what a person reading the
    /// frame would expect and it leaves more of the mantissa's range spare.
    pub fn from_quantity(q: Quantity) -> Result<Self, WireError> {
        if !q.value.is_finite() {
            return Err(WireError::NotFinite);
        }

        let mut scale = 1.0_f64;
        let mut exponent = 0_i8;

        // Purely relative, with no absolute floor. A floor was tried and was
        // wrong: it let a value far below any resolution round to zero and
        // still count as exact, which is the silent loss this check exists to
        // catch. Zero itself needs no floor, since its allowance and its error
        // are both zero.
        let allowed = TOLERANCE * q.value.abs();

        while exponent >= MIN_EXPONENT {
            let rounded = (q.value * scale).round();

            if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
                return Err(WireError::OutOfRange(q.value));
            }

            let back = rounded / scale;
            if (back - q.value).abs() <= allowed {
                return Ok(Num {
                    mantissa: rounded as i32,
                    exponent,
                    unit: q.unit,
                });
            }

            scale *= 10.0;
            exponent -= 1;
        }

        Err(WireError::NotRepresentable(q.value))
    }

    pub fn to_quantity(self) -> Quantity {
        Quantity::new(
            f64::from(self.mantissa) * 10_f64.powi(i32::from(self.exponent)),
            self.unit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_shortest_exact_form() {
        for (value, mantissa, exponent) in [
            (36.5_f64, 365_i32, -1_i8),
            (90.0, 90, 0),
            (0.0, 0, 0),
            (-4.25, -425, -2),
            (0.1, 1, -1),
            (1000.0, 1000, 0),
        ] {
            let n = Num::from_quantity(Quantity::new(value, Unit::G)).expect("representable");
            assert_eq!(
                (n.mantissa, n.exponent),
                (mantissa, exponent),
                "{value} should encode as {mantissa}e{exponent}"
            );
        }
    }

    /// The property that matters more than the chosen exponent: whatever form
    /// is picked, reading it back gives the value that went in.
    #[test]
    fn round_trips_without_drift() {
        for value in [36.5_f64, 0.1, 92.0, -4.25, 0.0, 2.4, 1e6, 0.001] {
            let q = Quantity::new(value, Unit::C);
            let back = Num::from_quantity(q).expect("representable").to_quantity();
            assert!(
                (back.value - value).abs() < 1e-9,
                "{value} came back as {}",
                back.value
            );
            assert_eq!(back.unit, Unit::C);
        }
    }

    /// 0.1 has no exact binary representation, so scaling it by ten lands just
    /// past one. Without the tolerance this is the value that would be refused.
    #[test]
    fn the_classic_inexact_value_survives() {
        let n = Num::from_quantity(Quantity::new(0.1, Unit::Ml)).expect("representable");
        assert_eq!((n.mantissa, n.exponent), (1, -1));
    }

    #[test]
    fn refuses_what_it_cannot_carry() {
        assert!(matches!(
            Num::from_quantity(Quantity::new(f64::NAN, Unit::G)),
            Err(WireError::NotFinite)
        ));
        assert!(matches!(
            Num::from_quantity(Quantity::new(f64::INFINITY, Unit::G)),
            Err(WireError::NotFinite)
        ));
        // Beyond what a 32 bit mantissa holds.
        assert!(matches!(
            Num::from_quantity(Quantity::new(1e18, Unit::G)),
            Err(WireError::OutOfRange(_))
        ));
    }

    /// A value too small to survive nine decimal places is refused rather
    /// than quietly becoming zero. A silently vanished actuation value is
    /// exactly the class of fault this representation exists to prevent.
    #[test]
    fn refuses_rather_than_rounding_to_nothing() {
        assert!(matches!(
            Num::from_quantity(Quantity::new(1e-12, Unit::G)),
            Err(WireError::NotRepresentable(_))
        ));
    }

    #[test]
    fn carries_the_unit_through() {
        let n = Num::from_quantity(Quantity::new(1.0, Unit::Step)).unwrap();
        assert_eq!(n.unit, Unit::Step);
        assert_eq!(n.to_quantity().unit, Unit::Step);
    }
}
