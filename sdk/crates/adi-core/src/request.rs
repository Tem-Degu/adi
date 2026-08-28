//! The request path: from structured intent to concrete targets.
//!
//! The correction path starts from a finished cycle and asks what went wrong.
//! This one starts from a person asking for something and works out what that
//! means in numbers. They meet at the planner.
//!
//! Nothing here talks to a model. The model's only job on this path is to
//! produce the [`Intent`], and by the time it arrives here it has been checked
//! against a closed schema. Everything below is arithmetic and lookups, which
//! is why "half the sugar of last time" can be resolved without trusting
//! anyone's memory or anyone's division.

use std::collections::BTreeMap;

use adi_protocol::ids::VariableId;
use adi_protocol::intent::{Intent, IntentError, Reference, TargetSpec};
use adi_protocol::units::Quantity;
use adi_protocol::Knowledge;

use crate::domain::Diagnosis;
use crate::memory::Memory;
use crate::plan::{Candidate, Planner};

/// A request with every target reduced to an absolute quantity.
#[derive(Debug, Clone)]
pub struct ResolvedRequest {
    pub style: Option<String>,
    pub material: Option<String>,
    pub targets: BTreeMap<VariableId, Quantity>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum RequestError {
    #[error(transparent)]
    Intent(#[from] IntentError),
    #[error("'{variable}' is relative to a cycle that is not remembered")]
    NoReference { variable: String },
    #[error("'{variable}' was not set on the cycle it is relative to")]
    NotSetOnReference { variable: String },
    #[error("offset for '{variable}' is in {found:?}, but the remembered value is {expected:?}")]
    UnitMismatch {
        variable: String,
        expected: adi_protocol::units::Unit,
        found: adi_protocol::units::Unit,
    },
    #[error("resolving '{0}' produced a value that is not a usable number")]
    NotFinite(String),
    /// The rule this path exists to enforce.
    #[error(
        "'{variable}' expresses taste, but '{tool}' also moves '{process}'; \
         a preference change must not alter the process"
    )]
    PreferenceWouldTouchProcess {
        variable: String,
        tool: String,
        process: String,
    },
    #[error("nothing declared can move '{0}'")]
    NoCapableTool(String),
}

/// Reduce an intent to absolute targets, using what the machine remembers.
///
/// This is where a relative request becomes a number, and it is deliberately
/// the engine's job rather than the model's. The model said "half of last
/// time"; whether last time was 8 g is a fact, not an opinion.
pub fn resolve(intent: &Intent, memory: &dyn Memory) -> Result<ResolvedRequest, RequestError> {
    intent.validate()?;

    let mut targets = BTreeMap::new();
    for (variable, spec) in &intent.targets {
        let value = match spec {
            TargetSpec::Absolute(q) => *q,
            TargetSpec::Relative(rel) => {
                let reference = match &rel.of {
                    Reference::Last => memory.last(),
                    Reference::Cycle(id) => memory.get(id),
                }
                .ok_or_else(|| RequestError::NoReference {
                    variable: variable.0.clone(),
                })?;

                let base = reference.setting(variable).ok_or_else(|| {
                    RequestError::NotSetOnReference {
                        variable: variable.0.clone(),
                    }
                })?;

                match (rel.scale, rel.offset) {
                    (Some(scale), None) => Quantity::new(base.value * scale, base.unit),
                    (None, Some(offset)) => {
                        // Adding across units gives a number that means
                        // nothing, and it would be dispensed anyway.
                        if offset.unit != base.unit {
                            return Err(RequestError::UnitMismatch {
                                variable: variable.0.clone(),
                                expected: base.unit,
                                found: offset.unit,
                            });
                        }
                        Quantity::new(base.value + offset.value, base.unit)
                    }
                    // Neither means "the same as then", which is how "make it
                    // like Tuesday's" resolves.
                    (None, None) => base,
                    // Rejected by validate, restated here so this match cannot
                    // silently acquire a wrong default later.
                    (Some(_), Some(_)) => {
                        return Err(RequestError::Intent(IntentError::ScaleAndOffset(
                            variable.0.clone(),
                        )))
                    }
                }
            }
        };

        if !value.value.is_finite() {
            return Err(RequestError::NotFinite(variable.0.clone()));
        }
        targets.insert(variable.clone(), value);
    }

    Ok(ResolvedRequest {
        style: intent.style.clone(),
        material: intent.material.clone(),
        targets,
    })
}

/// Turn one resolved target into the diagnosis the planner already understands.
///
/// The request path and the correction path converge here, so a request goes
/// through exactly the same candidate derivation, clamping, envelope checks and
/// record keeping as an engine-initiated correction. A separate path for
/// requests would be a second place for those rules to be got wrong.
pub fn as_diagnosis(
    variable: &VariableId,
    target: Quantity,
    current: Option<Quantity>,
    knowledge: &Knowledge,
) -> Diagnosis {
    let delta = current.map(|c| target.value - c.value);
    let preference = knowledge.is_preference(&variable.0);
    Diagnosis {
        cause: if preference {
            "preference_adjustment".into()
        } else {
            "requested_change".into()
        },
        target_variable: Some(variable.clone()),
        desired_delta: delta,
        // A request is not an inference. The person said what they wanted.
        confidence: 1.0,
        explanation: String::new(),
    }
}

/// Candidates for a requested change, with the preference rule applied.
///
/// ADI-P 7.1: the engine treats additives as preference variables, never as
/// process variables, and will not attempt to correct a taste fault by changing
/// them. This is the same rule seen from the other side. Someone asking for
/// less sugar has asked about taste, and a tool that would also move grind or
/// temperature to achieve it has changed the drink they did not ask about.
///
/// The check is on every variable a candidate touches, not only the one it was
/// selected for, because a tool's side effects are exactly where this would
/// otherwise leak.
pub fn candidates_for(
    planner: &Planner<'_>,
    knowledge: &Knowledge,
    variable: &VariableId,
) -> Result<Vec<Candidate>, RequestError> {
    let all = planner.candidates(variable);
    if all.is_empty() {
        return Err(RequestError::NoCapableTool(variable.0.clone()));
    }
    if !knowledge.is_preference(&variable.0) {
        return Ok(all);
    }

    let mut kept = Vec::new();
    let mut rejected: Option<RequestError> = None;
    for candidate in all {
        let touched = planner.affected_variables(&candidate);
        match touched.iter().find(|v| !knowledge.is_preference(&v.0)) {
            None => kept.push(candidate),
            Some(process) => {
                rejected.get_or_insert(RequestError::PreferenceWouldTouchProcess {
                    variable: variable.0.clone(),
                    tool: candidate.tool.0.clone(),
                    process: process.0.clone(),
                });
            }
        }
    }

    if kept.is_empty() {
        // Every capable tool would have altered the process. Refusing is the
        // right answer: the machine cannot do what was asked without changing
        // something the person did not ask about, and it should say so rather
        // than do it.
        return Err(rejected.unwrap_or(RequestError::NoCapableTool(variable.0.clone())));
    }
    Ok(kept)
}
