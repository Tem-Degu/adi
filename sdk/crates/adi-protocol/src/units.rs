//! Units and bounded numeric values.
//!
//! Every numeric value crossing the protocol carries its unit. ADI-P forbids
//! unitless numbers outside `Index` and `Count`, so the type system enforces it
//! rather than a convention.

use serde::{Deserialize, Serialize};

/// The closed unit vocabulary from ADI-P section 4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    G,
    GPerS,
    Ml,
    MlPerS,
    C,
    Bar,
    S,
    Ms,
    Step,
    KgF,
    Pct,
    /// Dimensionless ratio. Permitted only where the registry declares it.
    None,
}

impl Unit {
    /// Two units are comparable when arithmetic between them is meaningful.
    pub fn compatible_with(self, other: Unit) -> bool {
        self == other
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
