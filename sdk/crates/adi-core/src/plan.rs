//! Correction planning.
//!
//! Given a diagnosis that wants a variable moved, work out which declared tool
//! moves it and by how much. The magnitude always comes from here, never from a
//! model, at every autonomy level. That is the invariant ADI-P 9.3 rests on.

use adi_protocol::ids::{SemanticId, VariableId};
use adi_protocol::skill::Skill;
use adi_protocol::units::Quantity;
use adi_protocol::Declaration;

use crate::domain::{Diagnosis, Domain};
use crate::CoreError;

/// A planned adjustment to one tool parameter.
#[derive(Debug, Clone)]
pub struct Correction {
    pub tool: SemanticId,
    pub parameter: String,
    /// Signed change to apply to the current setting.
    pub delta: Quantity,
    /// Absolute value to send, after clamping.
    pub absolute: Quantity,
    /// Which variable this is intended to move.
    pub variable: VariableId,
    /// Confidence carried from the skill response, scaled by diagnosis
    /// confidence.
    pub confidence: f64,
    /// Set when the skill declares retention: the engine should purge or
    /// discount the next cycle rather than trusting it.
    pub requires_purge: Option<SemanticId>,
}

pub struct Planner<'a> {
    pub declaration: &'a Declaration,
    pub skill: Option<&'a Skill>,
    pub domain: &'a dyn Domain,
}

impl<'a> Planner<'a> {
    pub fn new(
        declaration: &'a Declaration,
        skill: Option<&'a Skill>,
        domain: &'a dyn Domain,
    ) -> Self {
        Self {
            declaration,
            skill,
            domain,
        }
    }

    /// Tools that can move `variable`, ranked most confident first.
    ///
    /// At D0 the engine picks the head of this list. At D1 the model picks from
    /// it. Either way the list itself is derived from declared capability and
    /// declared behaviour, so a model can only ever choose something the
    /// machine really supports.
    pub fn candidates(&self, variable: &VariableId) -> Vec<Candidate> {
        let mut out = Vec::new();
        for tool in self.declaration.engine_tools() {
            let Some(skill) = self.skill else {
                continue;
            };
            for response in skill.responses_for(&tool.id) {
                if &response.variable != variable {
                    continue;
                }
                if response.per_unit == 0.0 {
                    continue;
                }
                // A parameter with no declared range can never be planned
                // against, so offering it as a candidate would only produce a
                // failure later. The unit is carried here because a model
                // supplying a magnitude at D2 has to be told what it is
                // supplying: a bare number is not a quantity.
                let Some(range) = tool
                    .parameters
                    .get(&response.parameter)
                    .and_then(|p| p.range())
                else {
                    continue;
                };
                out.push(Candidate {
                    tool: tool.id.clone(),
                    parameter: response.parameter.clone(),
                    variable: variable.clone(),
                    per_unit: response.per_unit,
                    confidence: response.confidence,
                    param_unit: range.unit,
                    description: tool.description.clone(),
                });
            }
        }
        out.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// Candidates for several variables at once, ranked most confident first.
    ///
    /// What a request needs. "Weaker and smaller than last time" is two goals
    /// on two variables, and a selector shown only one variable's tools could
    /// never propose a plan that satisfies both.
    pub fn candidates_for_all(&self, variables: &[VariableId]) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        for variable in variables {
            for candidate in self.candidates(variable) {
                // A tool and parameter can appear under two goals if the skill
                // declares a response for each. Keeping both would let one
                // physical action be planned twice.
                if out
                    .iter()
                    .any(|c| c.tool == candidate.tool && c.parameter == candidate.parameter)
                {
                    continue;
                }
                out.push(candidate);
            }
        }
        out.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// Compute the correction for a chosen candidate.
    ///
    /// `current` is the parameter's present value. The magnitude comes from the
    /// skill's declared response, scaled down when confidence is low, then
    /// clamped to the declared range.
    pub fn plan(
        &self,
        diagnosis: &Diagnosis,
        candidate: &Candidate,
        current: Quantity,
    ) -> Result<Correction, CoreError> {
        let variable = diagnosis
            .target_variable
            .clone()
            .ok_or_else(|| CoreError::NoCapableTool("none requested".into()))?;

        let desired = diagnosis.desired_delta.unwrap_or(0.0);

        let tool = self
            .declaration
            .tool(&candidate.tool)
            .ok_or_else(|| CoreError::NoCapableTool(candidate.tool.0.clone()))?;
        let param = tool
            .parameters
            .get(&candidate.parameter)
            .ok_or_else(|| CoreError::NoResponse {
                tool: candidate.tool.0.clone(),
                variable: variable.0.clone(),
            })?;
        let range = param.range().ok_or_else(|| CoreError::NoResponse {
            tool: candidate.tool.0.clone(),
            variable: variable.0.clone(),
        })?;

        // Everything below produces a number that will be sent to hardware, so
        // the inputs are checked first rather than trusted.

        // `current` is the parameter's present value and must be in the
        // parameter's own unit. Arithmetic across units yields a number that
        // means nothing, and it would be sent anyway.
        if current.unit != range.unit {
            return Err(CoreError::UnitMismatch {
                parameter: candidate.parameter.clone(),
                expected: range.unit,
                found: current.unit,
            });
        }

        // A response of zero says the tool does not move the variable. Dividing
        // by it gives infinity, and clamping infinity gives the end of the
        // range, so the machine would be driven to its limit. `candidates`
        // filters these, but `Candidate` is constructible by any caller and at
        // D1 the selection comes from a model.
        if candidate.per_unit == 0.0 || !candidate.per_unit.is_finite() {
            return Err(CoreError::NoResponse {
                tool: candidate.tool.0.clone(),
                variable: variable.0.clone(),
            });
        }

        if !current.value.is_finite() || !desired.is_finite() {
            return Err(CoreError::NotFinite {
                parameter: candidate.parameter.clone(),
            });
        }

        // How far to move the parameter to achieve the desired change in the
        // variable. Signed division: the response already encodes direction.
        let raw_delta = desired / candidate.per_unit;

        // Low confidence means smaller steps. Overshooting costs a whole cycle
        // and, in a slow domain, possibly a whole week.
        let scaled = raw_delta * candidate.confidence.clamp(0.1, 1.0) * diagnosis.confidence;

        let absolute_value = range.clamp(current.value + scaled);

        // Last gate before the value leaves the engine. A non-finite confidence
        // on either side could still have produced one.
        if !absolute_value.is_finite() {
            return Err(CoreError::NotFinite {
                parameter: candidate.parameter.clone(),
            });
        }
        let applied_delta = absolute_value - current.value;

        let requires_purge = self
            .skill
            .and_then(|s| s.retention_for(&candidate.tool))
            .and_then(|r| r.remedy_tool.clone());

        Ok(Correction {
            tool: candidate.tool.clone(),
            parameter: candidate.parameter.clone(),
            delta: Quantity::new(applied_delta, range.unit),
            absolute: Quantity::new(absolute_value, range.unit),
            variable,
            confidence: candidate.confidence * diagnosis.confidence,
            requires_purge,
        })
    }

    /// Take a magnitude chosen elsewhere and make it safe to send. D2.
    ///
    /// The value is the absolute target for the parameter, not a delta, because
    /// a delta is ambiguous without knowing what it is relative to and the
    /// caller may not have observed the machine.
    ///
    /// Nothing here trusts the number. It must be in the parameter's declared
    /// unit, it must be finite, and it is clamped to the declared range and
    /// snapped to the declared resolution exactly as a computed value would be.
    /// The appliance then validates it again independently, as ADI-P 9.2
    /// requires, so this is the second of three gates rather than the only one.
    ///
    /// `variable` is passed rather than taken from a diagnosis because a value
    /// may arrive from a request that diagnosed nothing.
    pub fn plan_value(
        &self,
        variable: &VariableId,
        candidate: &Candidate,
        current: Quantity,
        requested: Quantity,
    ) -> Result<Correction, CoreError> {
        let tool = self
            .declaration
            .tool(&candidate.tool)
            .ok_or_else(|| CoreError::NoCapableTool(candidate.tool.0.clone()))?;
        let param = tool
            .parameters
            .get(&candidate.parameter)
            .ok_or_else(|| CoreError::NoResponse {
                tool: candidate.tool.0.clone(),
                variable: variable.0.clone(),
            })?;
        let range = param.range().ok_or_else(|| CoreError::NoResponse {
            tool: candidate.tool.0.clone(),
            variable: variable.0.clone(),
        })?;

        for (label, q) in [("requested", requested), ("current", current)] {
            if q.unit != range.unit {
                return Err(CoreError::UnitMismatch {
                    parameter: format!("{} ({label})", candidate.parameter),
                    expected: range.unit,
                    found: q.unit,
                });
            }
            if !q.value.is_finite() {
                return Err(CoreError::NotFinite {
                    parameter: candidate.parameter.clone(),
                });
            }
        }

        let absolute_value = range.clamp(requested.value);
        if !absolute_value.is_finite() {
            return Err(CoreError::NotFinite {
                parameter: candidate.parameter.clone(),
            });
        }

        let requires_purge = self
            .skill
            .and_then(|s| s.retention_for(&candidate.tool))
            .and_then(|r| r.remedy_tool.clone());

        Ok(Correction {
            tool: candidate.tool.clone(),
            parameter: candidate.parameter.clone(),
            delta: Quantity::new(absolute_value - current.value, range.unit),
            absolute: Quantity::new(absolute_value, range.unit),
            variable: variable.clone(),
            // The manufacturer's confidence in the response still applies. A
            // value chosen by a model does not become better attested for
            // having been chosen confidently.
            confidence: candidate.confidence,
            requires_purge,
        })
    }

    /// Every variable one step touches. ADI-P 9.5.
    ///
    /// Three sources, and all three are needed. The tool's declared `effects`
    /// are the physical things it changes, such as a position. The skill's
    /// responses say which domain variable moving that position actually moves,
    /// which is usually the reason the step was chosen and is not derivable
    /// from the declaration alone. And `also_affects` covers the rest.
    ///
    /// Leaving out the responses made the check useless in the ordinary case: a
    /// grinder declares it affects "position" and a doser declares it affects
    /// "input_mass", those do not intersect, and yet both were selected because
    /// they move the same shot time.
    pub fn affected_variables(&self, candidate: &Candidate) -> Vec<VariableId> {
        let Some(tool) = self.declaration.tool(&candidate.tool) else {
            return Vec::new();
        };
        let mut out: Vec<VariableId> = tool.effects.clone();
        if let Some(skill) = self.skill {
            for response in skill.responses_for(&candidate.tool) {
                if response.parameter == candidate.parameter {
                    out.push(response.variable.clone());
                }
            }
            for effect in &tool.effects {
                for interaction in &skill.interactions {
                    if interaction.changing == *effect {
                        out.extend(interaction.also_affects.iter().cloned());
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }
}

/// A tool that can move a variable, with the skill's declared response.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub tool: SemanticId,
    pub parameter: String,
    /// The variable this candidate was derived to move.
    ///
    /// Implicit when a candidate list only ever covered one variable. A list
    /// spanning several, which is what a request needs, has to say which
    /// candidate serves which goal.
    pub variable: VariableId,
    /// Change in the variable per unit of the parameter. Signed.
    pub per_unit: f64,
    pub confidence: f64,
    /// The unit the parameter is declared in, which is not the unit of the
    /// variable it moves. Grind position is in steps; it moves shot time in
    /// seconds.
    pub param_unit: crate::plan::Unit,
    /// The manufacturer's description of the tool, where they wrote one.
    ///
    /// Carried so a model choosing between candidates can read it. Nothing in
    /// the engine reads it, and the declaration's documentation states why.
    pub description: Option<String>,
}

pub use adi_protocol::units::Unit;
