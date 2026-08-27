//! The domain-neutral reasoning loop.
//!
//! ```text
//!   observe a cycle
//!        -> compare against targets resolved through the knowledge layers
//!        -> diagnose a cause
//!        -> plan a correction from the skill's declared responses
//!        -> clamp, check, record
//!        -> learn
//! ```
//!
//! A cycle is a coffee extraction, a wash, a print, a bake. This crate must not
//! know which. See the rule in `sdk/README.md`.

pub mod cycle;
pub mod domain;
pub mod plan;
pub mod record;

pub use cycle::{Cycle, SubjectiveFeedback, Trace};
pub use domain::{Diagnosis, Domain};
pub use plan::{Correction, Planner};
pub use record::{DecisionRecord, SelectedBy, ValueProvenance, ValueSource};

use adi_protocol::knowledge::{Layer, ResolvedTargets};
use adi_protocol::ProtocolError;

/// Resolve targets across the four layers, narrowing at each step.
///
/// Constraint monotonicity is enforced here, not merely documented: a layer
/// that widens a range is a validation error. See ADI-P 6.4.
pub fn resolve_targets(
    base: ResolvedTargets,
    contributions: &[(Layer, ResolvedTargets)],
) -> Result<ResolvedTargets, ProtocolError> {
    let mut resolved = base;
    for (layer, contribution) in contributions {
        for (key, attributed) in contribution.iter() {
            resolved.apply(key, attributed.value.clone(), *layer)?;
        }
    }
    Ok(resolved)
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no tool available that affects '{0}'")]
    NoCapableTool(String),
    #[error("profile '{profile}' does not model variable '{variable}'")]
    UnmodelledVariable { profile: String, variable: String },
    #[error("skill declares no response for tool '{tool}' on '{variable}', and the profile has no default")]
    NoResponse { tool: String, variable: String },
    #[error(transparent)]
    Safety(#[from] adi_safety::SafetyViolation),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
}
