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
    /// Correlates the response and the decision record.
    pub invocation_id: String,
}

impl Invocation {
    pub fn new(tool: SemanticId, invocation_id: impl Into<String>) -> Self {
        Self {
            tool,
            numeric_arguments: BTreeMap::new(),
            text_arguments: BTreeMap::new(),
            invocation_id: invocation_id.into(),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RejectReason {
    OutOfRange,
    EnvelopeViolation,
    PreconditionFailed,
    Interlock,
    Busy,
    NotAuthorised,
    Fault,
    /// A stateful constraint would be breached by this call in context.
    InvariantViolation,
    RateLimited,
    ResourceExhausted,
}
