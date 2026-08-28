//! Safe Action Protocol.
//!
//! The principle: an AI may decide freely, but may act only through declared
//! actions that are individually bounded and safe.
//!
//! The part that is not automatic: individually safe actions can compose into
//! an unsafe sequence. Three valid dispense calls overflow a cup. Fifty valid
//! brews exceed a boiler's thermal design. Per-call limits cannot express
//! either, because the constraint exists across time rather than within one
//! call.
//!
//! So this crate enforces two things:
//!
//! 1. **Per-invocation limits.** Parameter ranges and the declared envelope.
//! 2. **Stateful constraints.** Invariants, rate limits and resource
//!    accounting, evaluated against accumulated state.
//!
//! The engine checking these does not make the appliance's own enforcement
//! optional. ADI-P requires the device to enforce independently, and this crate
//! exists so the engine does not knowingly send something the device will
//! reject.
//!
//! Domain-neutral. Nothing here knows what any quantity means.

use std::collections::BTreeMap;

use adi_protocol::invocation::{Invocation, RejectReason};
use adi_protocol::units::Quantity;
use adi_protocol::{Declaration, EnvelopeSpec, SemanticId};

/// Accumulated machine state, as far as the engine can model it.
#[derive(Debug, Clone, Default)]
pub struct SafetyState {
    /// Accumulator per invariant id.
    accumulated: BTreeMap<String, f64>,
    /// Remaining capacity per resource id.
    resources: BTreeMap<String, f64>,
    /// Monotonic timestamps of recent invocations, per tool.
    history: BTreeMap<String, Vec<f64>>,
    /// Seconds since an arbitrary epoch. Injected rather than read from a
    /// clock, so decisions stay reproducible.
    pub now_s: f64,
}

impl SafetyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance_to(&mut self, now_s: f64) {
        self.now_s = now_s;
    }

    pub fn accumulated(&self, invariant_id: &str) -> f64 {
        self.accumulated.get(invariant_id).copied().unwrap_or(0.0)
    }

    pub fn reset(&mut self, invariant_id: &str) {
        self.accumulated.remove(invariant_id);
    }

    pub fn set_resource(&mut self, resource_id: &str, remaining: f64) {
        self.resources.insert(resource_id.to_string(), remaining);
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SafetyViolation {
    #[error("parameter '{parameter}' value {value} is outside the declared range")]
    OutOfRange { parameter: String, value: f64 },
    #[error("parameter '{parameter}' breaches hard limit on '{limit}'")]
    EnvelopeViolation { parameter: String, limit: String },
    #[error("invariant '{invariant}' would reach {projected} against a limit of {limit}")]
    Invariant {
        invariant: String,
        projected: f64,
        limit: f64,
    },
    #[error("rate limit '{limit}' exceeded for tool '{tool}'")]
    RateLimited { limit: String, tool: String },
    #[error("resource '{resource}' exhausted")]
    ResourceExhausted { resource: String },
    #[error("interlock '{interlock}' blocks tool '{tool}'")]
    Interlock { interlock: String, tool: String },
    #[error("unit mismatch on parameter '{parameter}'")]
    UnitMismatch { parameter: String },
    #[error("tool '{0}' is not declared")]
    UnknownTool(String),
}

impl SafetyViolation {
    /// The reason code the appliance would return for this violation.
    pub fn reject_reason(&self) -> RejectReason {
        match self {
            SafetyViolation::OutOfRange { .. } | SafetyViolation::UnitMismatch { .. } => {
                RejectReason::OutOfRange
            }
            SafetyViolation::EnvelopeViolation { .. } => RejectReason::EnvelopeViolation,
            SafetyViolation::Invariant { .. } => RejectReason::InvariantViolation,
            SafetyViolation::RateLimited { .. } => RejectReason::RateLimited,
            SafetyViolation::ResourceExhausted { .. } => RejectReason::ResourceExhausted,
            SafetyViolation::Interlock { .. } => RejectReason::Interlock,
            SafetyViolation::UnknownTool(_) => RejectReason::PreconditionFailed,
        }
    }
}

/// Checks invocations against a declaration and accumulated state.
pub struct Guard<'a> {
    declaration: &'a Declaration,
    open_interlocks: Vec<String>,
}

impl<'a> Guard<'a> {
    pub fn new(declaration: &'a Declaration) -> Self {
        Self {
            declaration,
            open_interlocks: Vec::new(),
        }
    }

    /// Interlocks currently open, reported by the appliance.
    pub fn set_open_interlocks(&mut self, ids: Vec<String>) {
        self.open_interlocks = ids;
    }

    fn envelope(&self) -> &EnvelopeSpec {
        &self.declaration.envelope
    }

    /// Clamp an argument to the declared range before it is ever sent.
    ///
    /// Clamping here is correct. Clamping in the appliance is not: the device
    /// must reject, so an engine defect stays visible. See ADI-P 4.6.
    pub fn clamp(&self, tool: &SemanticId, parameter: &str, value: Quantity) -> Option<Quantity> {
        let tool = self.declaration.tool(tool)?;
        let param = tool.parameters.get(parameter)?;
        let range = param.range()?;
        if !range.unit.compatible_with(value.unit) {
            return None;
        }
        Some(Quantity::new(range.clamp(value.value), range.unit))
    }

    /// Full check of one invocation in context.
    pub fn check(
        &self,
        invocation: &Invocation,
        state: &SafetyState,
    ) -> Result<(), SafetyViolation> {
        let tool = self
            .declaration
            .tool(&invocation.tool)
            .ok_or_else(|| SafetyViolation::UnknownTool(invocation.tool.0.clone()))?;

        for interlock in &self.envelope().interlocks {
            if !self.open_interlocks.contains(&interlock.id) {
                continue;
            }
            let blocked = interlock
                .blocks
                .iter()
                .any(|b| b == "*" || b == invocation.tool.as_str());
            if blocked {
                return Err(SafetyViolation::Interlock {
                    interlock: interlock.id.clone(),
                    tool: invocation.tool.0.clone(),
                });
            }
        }

        for (name, quantity) in &invocation.numeric_arguments {
            let param = tool
                .parameters
                .get(name)
                .ok_or_else(|| SafetyViolation::OutOfRange {
                    parameter: name.clone(),
                    value: quantity.value,
                })?;
            if let Some(range) = param.range() {
                if !range.unit.compatible_with(quantity.unit) {
                    return Err(SafetyViolation::UnitMismatch {
                        parameter: name.clone(),
                    });
                }
                if !range.contains(quantity.value) {
                    return Err(SafetyViolation::OutOfRange {
                        parameter: name.clone(),
                        value: quantity.value,
                    });
                }
            }
            for limit in &self.envelope().limits {
                if &limit.parameter != name {
                    continue;
                }
                let breaches_min = limit.hard_min.is_some_and(|m| quantity.value < m);
                let breaches_max = limit.hard_max.is_some_and(|m| quantity.value > m);
                if breaches_min || breaches_max {
                    return Err(SafetyViolation::EnvelopeViolation {
                        parameter: name.clone(),
                        limit: limit.parameter.clone(),
                    });
                }
            }
        }

        self.check_invariants(invocation, state)?;
        self.check_rate_limits(invocation, state)?;
        self.check_resources(invocation, state)?;
        Ok(())
    }

    /// The composition check. This is what makes the Safe Action Protocol claim
    /// true rather than merely plausible.
    fn check_invariants(
        &self,
        invocation: &Invocation,
        state: &SafetyState,
    ) -> Result<(), SafetyViolation> {
        for invariant in &self.envelope().invariants {
            let Some(param_name) = invariant.accumulates.get(invocation.tool.as_str()) else {
                continue;
            };
            let Some(contribution) = invocation.numeric_arguments.get(param_name) else {
                continue;
            };

            // Adding grams to a millilitre total produces a number that means
            // nothing, and an accumulator that means nothing stops enforcing
            // the limit it exists to enforce. Refuse rather than sum.
            if !contribution.unit.compatible_with(invariant.unit) {
                return Err(SafetyViolation::UnitMismatch {
                    parameter: param_name.clone(),
                });
            }

            let projected = state.accumulated(&invariant.id) + contribution.value;
            if let Some(max) = invariant.max {
                if projected > max {
                    return Err(SafetyViolation::Invariant {
                        invariant: invariant.id.clone(),
                        projected,
                        limit: max,
                    });
                }
            }
            if let Some(min) = invariant.min {
                if projected < min {
                    return Err(SafetyViolation::Invariant {
                        invariant: invariant.id.clone(),
                        projected,
                        limit: min,
                    });
                }
            }
        }
        Ok(())
    }

    fn check_rate_limits(
        &self,
        invocation: &Invocation,
        state: &SafetyState,
    ) -> Result<(), SafetyViolation> {
        for limit in &self.envelope().rate_limits {
            if !limit
                .tools
                .iter()
                .any(|t| t == "*" || t == invocation.tool.as_str())
            {
                continue;
            }
            let empty = Vec::new();
            let history = state
                .history
                .get(invocation.tool.as_str())
                .unwrap_or(&empty);

            if let Some(min_interval) = limit.min_interval_s {
                if let Some(last) = history.last() {
                    if state.now_s - last < min_interval {
                        return Err(SafetyViolation::RateLimited {
                            limit: limit.id.clone(),
                            tool: invocation.tool.0.clone(),
                        });
                    }
                }
            }

            if let (Some(max), Some(window)) = (limit.max_invocations, limit.window_s) {
                let recent = history
                    .iter()
                    .filter(|t| state.now_s - **t <= window)
                    .count();
                if recent as u32 >= max {
                    return Err(SafetyViolation::RateLimited {
                        limit: limit.id.clone(),
                        tool: invocation.tool.0.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn check_resources(
        &self,
        invocation: &Invocation,
        state: &SafetyState,
    ) -> Result<(), SafetyViolation> {
        for resource in &self.envelope().resources {
            let Some(param_name) = resource.consumed_by.get(invocation.tool.as_str()) else {
                continue;
            };
            let Some(consumption) = invocation.numeric_arguments.get(param_name) else {
                continue;
            };

            if !consumption.unit.compatible_with(resource.unit) {
                return Err(SafetyViolation::UnitMismatch {
                    parameter: param_name.clone(),
                });
            }

            let remaining = state
                .resources
                .get(&resource.id)
                .copied()
                .unwrap_or(resource.capacity.unwrap_or(f64::INFINITY));
            if consumption.value > remaining {
                return Err(SafetyViolation::ResourceExhausted {
                    resource: resource.id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Record an accepted invocation so later checks see it.
    pub fn commit(&self, invocation: &Invocation, state: &mut SafetyState) {
        for invariant in &self.envelope().invariants {
            if let Some(param_name) = invariant.accumulates.get(invocation.tool.as_str()) {
                if let Some(contribution) = invocation.numeric_arguments.get(param_name) {
                    // Mirrors the check above. If these two disagreed about
                    // what counts, an invocation could pass the check and then
                    // record something different.
                    if contribution.unit.compatible_with(invariant.unit) {
                        *state
                            .accumulated
                            .entry(invariant.id.clone())
                            .or_insert(0.0) += contribution.value;
                    }
                }
            }
            if invariant
                .reset_on
                .iter()
                .any(|t| t == invocation.tool.as_str())
            {
                state.accumulated.remove(&invariant.id);
            }
        }
        state
            .history
            .entry(invocation.tool.0.clone())
            .or_default()
            .push(state.now_s);
    }
}
