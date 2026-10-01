//! Units and bounded numeric values.
//!
//! Every numeric value crossing the protocol carries its unit. ADI-P forbids
//! unitless numbers outside `Index` and `Count`, so the type system enforces it
//! rather than a convention.

use serde::{Deserialize, Serialize};

/// The closed unit vocabulary from ADI-P section 4.2.
///
/// The discriminants are the wire values from ADI-P 10.2.3. A quantity carries
/// its unit across the wire, so these numbers are normative and are declared
/// here rather than in a conversion table: adding a unit without choosing its
/// number then fails to compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Unit {
    G = 1,
    GPerS = 2,
    Ml = 3,
    MlPerS = 4,
    C = 5,
    Bar = 6,
    S = 7,
    Ms = 8,
    Step = 9,
    KgF = 10,
    Pct = 11,
    /// Dimensionless ratio. Permitted only where the registry declares it.
    None = 0,
}

impl Unit {
    /// Two units are comparable when arithmetic between them is meaningful.
    pub fn compatible_with(self, other: Unit) -> bool {
        self == other
    }

    /// The ADI-P 10.2.3 wire value.
    pub fn as_wire(self) -> u8 {
        self as u8
    }

    /// Parse a wire value. `None` for any number this version does not define,
    /// which a decoder must reject rather than guess at.
    pub fn from_wire(value: u8) -> Option<Self> {
        use Unit::*;
        Some(match value {
            0 => None,
            1 => G,
            2 => GPerS,
            3 => Ml,
            4 => MlPerS,
            5 => C,
            6 => Bar,
            7 => S,
            8 => Ms,
            9 => Step,
            10 => KgF,
            11 => Pct,
            _ => return Option::None,
        })
    }
}

/// A value with its unit attached. Constructed only through the protocol
/// parsers, so an unlabelled number cannot reach the reasoning layers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
}

impl Quantity {
    pub fn new(value: f64, unit: Unit) -> Self {
        Self { value, unit }
    }
}

/// A declared operating range for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
    #[serde(default)]
    pub resolution: Option<f64>,
    pub unit: Unit,
}

impl Range {
    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }

    /// Narrow to the intersection with `other`. Layer resolution only ever
    /// narrows, never widens. See ADI-P 6.4.
    pub fn intersect(&self, other: &Range) -> Option<Range> {
        if !self.unit.compatible_with(other.unit) {
            return None;
        }
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        if min > max {
            return None;
        }
        Some(Range {
            min,
            max,
            resolution: self.resolution.or(other.resolution),
            unit: self.unit,
        })
    }

    /// Clamp and snap to the declared resolution.
    /// Whether a value lies outside the declared range, and so cannot be
    /// reached by this parameter at all.
    ///
    /// Distinct from what `clamp` does, which is both limiting and snapping to
    /// resolution. Snapping 21.4 to 21 on a parameter declared in whole steps
    /// is ordinary and means nothing is wrong. Being held at the end of the
    /// range means the mechanism cannot go where the engine wanted, and that is
    /// worth recording: without it the machine reports having acted, and the
    /// person cannot tell a small adjustment from no adjustment at all.
    pub fn exceeds(&self, value: f64) -> bool {
        value < self.min || value > self.max
    }

    pub fn clamp(&self, value: f64) -> f64 {
        let clamped = value.clamp(self.min, self.max);
        match self.resolution {
            Some(step) if step > 0.0 => {
                let snapped = (clamped / step).round() * step;
                snapped.clamp(self.min, self.max)
            }
            _ => clamped,
        }
    }
}

/// Direction of a mechanism where direction carries domain meaning.
/// The engine must never infer this. See ADI-P 4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Monotonic {
    HigherIsMore,
    HigherIsLess,
    /// Domain-specific direction, resolved by the profile.
    ProfileDefined,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire numbering is normative in ADI-P 10.2.3, so it is asserted
    /// against the literal numbers rather than against itself. A renumbering
    /// that keeps round-tripping would otherwise pass.
    #[test]
    fn unit_wire_values_match_the_specification() {
        for (unit, wire) in [
            (Unit::None, 0u8),
            (Unit::G, 1),
            (Unit::GPerS, 2),
            (Unit::Ml, 3),
            (Unit::MlPerS, 4),
            (Unit::C, 5),
            (Unit::Bar, 6),
            (Unit::S, 7),
            (Unit::Ms, 8),
            (Unit::Step, 9),
            (Unit::KgF, 10),
            (Unit::Pct, 11),
        ] {
            assert_eq!(unit.as_wire(), wire, "{unit:?} should encode as {wire}");
            assert_eq!(Unit::from_wire(wire), Some(unit), "{wire} should decode");
        }
    }

    #[test]
    fn an_undefined_unit_is_refused_rather_than_guessed() {
        assert_eq!(Unit::from_wire(12), None);
        assert_eq!(Unit::from_wire(255), None);
    }

    #[test]
    fn clamp_snaps_to_the_declared_resolution() {
        let range = Range {
            min: 0.0,
            max: 60.0,
            resolution: Some(1.0),
            unit: Unit::Step,
        };
        assert_eq!(range.clamp(21.4), 21.0);
        assert_eq!(range.clamp(-5.0), 0.0);
        assert_eq!(range.clamp(100.0), 60.0);
    }

    #[test]
    fn intersect_refuses_to_cross_units() {
        let steps = Range { min: 0.0, max: 10.0, resolution: None, unit: Unit::Step };
        let seconds = Range { min: 0.0, max: 10.0, resolution: None, unit: Unit::S };
        assert!(steps.intersect(&seconds).is_none());
    }
}
