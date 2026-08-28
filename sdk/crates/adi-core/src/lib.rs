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
pub mod memory;
pub mod plan;
pub mod record;
pub mod request;

pub use cycle::{Cycle, SubjectiveFeedback, Trace};
pub use domain::{Diagnosis, Domain};
pub use plan::{Correction, Planner};
pub use record::{DecisionRecord, SelectedBy, ValueProvenance, ValueSource};
pub use memory::{Memory, NoMemory, RecentCycles, Remembered};
pub use request::{RequestError, ResolvedRequest};

// Layer resolution lives in `adi_protocol::Knowledge`, which owns the layer
// stack and can therefore follow inheritance and attribute each effective value
// to the layer that produced it. An earlier `resolve_targets` here took
// already-resolved targets and could do neither.

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no tool available that affects '{0}'")]
    NoCapableTool(String),
    #[error("profile '{profile}' does not model variable '{variable}'")]
    UnmodelledVariable { profile: String, variable: String },
    #[error("skill declares no response for tool '{tool}' on '{variable}', and the profile has no default")]
    NoResponse { tool: String, variable: String },
    /// The present value of a parameter arrived in the wrong unit. Computing
    /// across units produces a number that means nothing and would be sent to
    /// hardware regardless.
    #[error("parameter '{parameter}' is declared in {expected:?}, but the current value is {found:?}")]
    UnitMismatch {
        parameter: String,
        expected: adi_protocol::units::Unit,
        found: adi_protocol::units::Unit,
    },
    /// An actuation value came out infinite or NaN. Always an engine defect,
    /// and refused rather than sent.
    #[error("computed a non-finite value for parameter '{parameter}'")]
    NotFinite { parameter: String },
    #[error(transparent)]
    Safety(#[from] adi_safety::SafetyViolation),
    #[error(transparent)]
    Protocol(#[from] adi_protocol::ProtocolError),
    #[error(transparent)]
    Knowledge(#[from] adi_protocol::KnowledgeError),
}
