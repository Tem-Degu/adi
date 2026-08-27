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
                out.push(Candidate {
                    tool: tool.id.clone(),
                    parameter: response.parameter.clone(),
                    per_unit: response.per_unit,
                    confidence: response.confidence,
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

        // How far to move the parameter to achieve the desired change in the
        // variable. Signed division: the response already encodes direction.
        let raw_delta = desired / candidate.per_unit;

        // Low confidence means smaller steps. Overshooting costs a whole cycle
        // and, in a slow domain, possibly a whole week.
        let scaled = raw_delta * candidate.confidence.clamp(0.1, 1.0) * diagnosis.confidence;

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

        let absolute_value = range.clamp(current.value + scaled);
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
}

/// A tool that can move a variable, with the skill's declared response.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub tool: SemanticId,
    pub parameter: String,
    /// Change in the variable per unit of the parameter. Signed.
    pub per_unit: f64,
    pub confidence: f64,
}
