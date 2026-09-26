//! Invocations and their outcomes. ADI-P section 9.2.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::SemanticId;
use crate::units::Quantity;

/// One tool call, fully resolved. Every numeric argument carries its unit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invocation {
    pub tool: SemanticId,
    #[serde(default)]
    pub numeric_arguments: BTreeMap<String, Quantity>,
    #[serde(default)]
    pub text_arguments: BTreeMap<String, String>,
    /// Correlates a response with its request, within one session, and nothing
    /// more. ADI-P 10.2.5 fixes this as a 32-bit integer because an appliance
    /// should not have to carry a heap string to match a reply.
    ///
    /// This is not the engine's audit identity. `DecisionRecord` holds that,
    /// it is richer, and it does not cross the wire.
    pub invocation_id: u32,
}

impl Invocation {
    pub fn new(tool: SemanticId, invocation_id: u32) -> Self {
        Self {
            tool,
            numeric_arguments: BTreeMap::new(),
            text_arguments: BTreeMap::new(),
            invocation_id,
        }
    }

    pub fn with_number(mut self, name: &str, value: Quantity) -> Self {
        self.numeric_arguments.insert(name.to_string(), value);
        self
    }

    pub fn with_text(mut self, name: &str, value: impl Into<String>) -> Self {
        self.text_arguments.insert(name.to_string(), value.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    Accepted,
    Running {
        #[serde(default)]
        progress_pct: Option<f64>,
    },
    Completed,
    Aborted {
        reason: String,
    },
    Rejected {
        reason: RejectReason,
        #[serde(default)]
        detail: Option<String>,
    },
}

/// Reason codes from ADI-P 9.2.
///
/// `OutOfRange` indicates an engine defect: the constraint layer should have
/// clamped before the invocation was sent. The appliance rejects rather than
/// clamping silently, because silent clamping hides the fault forever.
///
/// The discriminants are the wire values from ADI-P 10.2.6 and are normative.
/// They live on the type rather than in a conversion table so that adding a
/// variant without deciding its number is a compile error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(u8)]
pub enum RejectReason {
    OutOfRange = 1,
    EnvelopeViolation = 2,
    PreconditionFailed = 3,
    Interlock = 4,
    Busy = 5,
    NotAuthorised = 6,
    Fault = 7,
    /// A stateful constraint would be breached by this call in context.
    InvariantViolation = 8,
    RateLimited = 9,
    ResourceExhausted = 10,
    /// The named tool is not in the declaration.
    UnknownTool = 11,
    /// The bytes were not a decodable control message.
    Malformed = 12,
    /// A quantity carried a unit the declaration does not name for that
    /// parameter. Distinct from `OutOfRange` because a value in range with the
    /// wrong unit has a different cause from a value out of range.
    UnitMismatch = 13,
}

impl RejectReason {
    /// The ADI-P 10.2.6 wire value.
    pub fn as_wire(self) -> u8 {
        self as u8
    }

    /// Parse a wire value. `None` for 0, which is "no reason", and for any
    /// number this version does not define.
    pub fn from_wire(value: u8) -> Option<Self> {
        use RejectReason::*;
        Some(match value {
            1 => OutOfRange,
            2 => EnvelopeViolation,
            3 => PreconditionFailed,
            4 => Interlock,
            5 => Busy,
            6 => NotAuthorised,
            7 => Fault,
            8 => InvariantViolation,
            9 => RateLimited,
            10 => ResourceExhausted,
            11 => UnknownTool,
            12 => Malformed,
            13 => UnitMismatch,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As in `units`, the numbers are the contract, so they are written out.
    #[test]
    fn reason_wire_values_match_the_specification() {
        use RejectReason::*;
        for (reason, wire) in [
            (OutOfRange, 1u8),
            (EnvelopeViolation, 2),
            (PreconditionFailed, 3),
            (Interlock, 4),
            (Busy, 5),
            (NotAuthorised, 6),
            (Fault, 7),
            (InvariantViolation, 8),
            (RateLimited, 9),
            (ResourceExhausted, 10),
            (UnknownTool, 11),
            (Malformed, 12),
            (UnitMismatch, 13),
        ] {
            assert_eq!(reason.as_wire(), wire, "{reason:?} should encode as {wire}");
            assert_eq!(RejectReason::from_wire(wire), Some(reason));
        }
    }

    /// Zero means "no reason" and is carried by omitting the field, so it must
    /// not decode to a reason. A zeroed buffer reading as a real rejection
    /// would be the worst possible default.
    #[test]
    fn zero_is_not_a_reason() {
        assert_eq!(RejectReason::from_wire(0), None);
        assert_eq!(RejectReason::from_wire(14), None);
    }

    #[test]
    fn an_invocation_carries_units_with_its_numbers() {
        let call = Invocation::new(SemanticId::from("adi.example.actuator.start"), 7)
            .with_number("mass", Quantity::new(36.5, crate::units::Unit::G))
            .with_text("substance", "example");

        assert_eq!(call.invocation_id, 7);
        assert_eq!(call.numeric_arguments["mass"].unit, crate::units::Unit::G);
        assert_eq!(call.text_arguments["substance"], "example");
    }
}
