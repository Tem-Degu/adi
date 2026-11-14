//! The explain seam, served by a separate program.
//!
//! The fourth seam, and the last one to get a backend. `Seam::Explainer` has
//! been in the configuration vocabulary all along with nothing behind it, so
//! every build fell through to `RecordExplainer` - which is true, and flat.
//!
//! WHY A MODEL IS WORTH IT HERE AND NOT ELSEWHERE.
//!
//! The other three seams sit in front of an action. This one sits behind one.
//! By the time it is asked, the drink is poured and the record is written, so
//! there is nothing left to get wrong and nothing to wait for: a second of
//! thinking is invisible because the machine is already busy or already done.
//!
//! And it is the seam the whole idea rests on. A machine that adjusts itself is
//! a curiosity; a machine that can be asked WHY it did that, and answer from
//! the record rather than from a template, is the thing being argued for. The
//! record already holds every input, the layer behind each value and the rule
//! that fired. This turns that into a sentence.
//!
//! WHY IT CANNOT LIE. It is handed what happened, not what was intended. A
//! model writing an acknowledgement before the fact can promise a drink that
//! is then refused; a model writing from a record can only describe a record.
//! That asymmetry is why the acknowledgement is short and says nothing about
//! quantities, and why everything factual is said here instead.

use std::sync::Arc;

use adi_core::record::{DecisionRecord, SelectedBy, ValueSource};
use adi_language::{Explainer, LanguageError};
use serde::Serialize;

use crate::backend::ProcessBackend;
use crate::ModelError;

/// One value that fed the decision, flattened for the wire.
#[derive(Debug, Serialize)]
struct InputLine {
    variable: String,
    value: f64,
    unit: String,
    /// Which layer the figure came from: base, profile, manufacturer, operator.
    layer: String,
    /// How it was arrived at, in words a model can use.
    source: String,
}

#[derive(Debug, Serialize)]
struct ArgumentLine {
    parameter: String,
    value: f64,
    unit: String,
    /// Present where the envelope moved the value, which is worth saying out
    /// loud: a machine that was asked for more than it will do should admit it
    /// rather than quietly delivering less.
    #[serde(skip_serializing_if = "Option::is_none")]
    asked_for: Option<f64>,
}

#[derive(Debug, Serialize)]
struct Explain<'a> {
    /// What the engine diagnosed, as the profile named it.
    cause: &'a str,
    /// The rule that fired, where one did.
    #[serde(skip_serializing_if = "str::is_empty")]
    rule: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    /// engine, model, or operator. ADI-P 9.3 requirement 4 wants a record that
    /// says which, and a person asking "why did you do that" is owed it too.
    chose: &'a str,
    arguments: Vec<ArgumentLine>,
    inputs: Vec<InputLine>,
    /// What the domain rules wrote. Flat, and true, and the thing being
    /// rephrased rather than replaced: a model that contradicts this is wrong.
    note: &'a str,
}

#[derive(Debug, Serialize)]
struct ExplainRequest<'a> {
    explain: Explain<'a>,
}

#[derive(Debug, Serialize)]
struct Refusal<'a> {
    /// What the person asked for, in their terms.
    asked: &'a str,
    /// Why it was refused, in the engine's. Already accurate; this seam is
    /// only making it sound like a person said it.
    reason: &'a str,
}

#[derive(Debug, Serialize)]
struct RefusalRequest<'a> {
    refusal: Refusal<'a>,
}

/// Explains by asking a program, falling back to nothing.
pub struct ProcessExplainer {
    backend: Arc<ProcessBackend>,
}

impl ProcessExplainer {
    pub fn start(command: &str, args: &[String]) -> Result<Self, ModelError> {
        let backend = ProcessBackend::start(command, args).map_err(ModelError::Backend)?;
        Ok(Self::over(Arc::new(backend)))
    }

    /// Over a program already running, so this seam and the others can be the
    /// same process.
    pub fn over(backend: Arc<ProcessBackend>) -> Self {
        Self { backend }
    }

    fn ask(&self, line: &str) -> Result<String, LanguageError> {
        let raw = self
            .backend
            .exchange(line)
            .map_err(|e| LanguageError::Backend(e.to_string()))?;

        #[derive(serde::Deserialize)]
        struct Said {
            text: String,
        }

        let said: Said = crate::parse::json(&raw)
            .map_err(|e| LanguageError::Backend(e.to_string()))?;

        let text = said.text.trim().to_string();
        if text.is_empty() {
            // An empty sentence is not an explanation. Reported so the caller
            // can fall back to the record's own words rather than saying
            // nothing at all.
            return Err(LanguageError::Backend(
                "the explainer returned an empty sentence".into(),
            ));
        }
        Ok(text)
    }
}

fn layer_name(layer: adi_protocol::knowledge::Layer) -> &'static str {
    use adi_protocol::knowledge::Layer;
    match layer {
        Layer::Base => "base knowledge",
        Layer::Vendor => "the manufacturer",
        Layer::Deployment => "this deployment",
        // The protocol already has somewhere for a learned figure to live, and
        // saying so out loud is the point: a person asking why should be told
        // when the answer is "because you taught me".
        Layer::User => "something learned from you",
    }
}

fn source_name(source: &ValueSource) -> String {
    match source {
        ValueSource::SkillResponse { tool, parameter } => {
            format!("the declared response of {} to {}", tool.as_str(), parameter)
        }
        other => format!("{other:?}"),
    }
}

impl Explainer for ProcessExplainer {
    fn explain(&self, record: &DecisionRecord) -> Result<String, LanguageError> {
        let request = ExplainRequest {
            explain: Explain {
                cause: &record.cause,
                rule: &record.rule_id,
                tool: record.selected_tool.as_ref().map(|t| t.as_str().to_string()),
                chose: match record.selected_by {
                    SelectedBy::Engine => "the engine, from the declaration",
                    SelectedBy::Model => "the model, from what the engine offered",
                    SelectedBy::Operator => "the person, by naming it",
                },
                arguments: record
                    .arguments
                    .iter()
                    .map(|(parameter, quantity)| ArgumentLine {
                        parameter: parameter.clone(),
                        value: quantity.value,
                        unit: format!("{:?}", quantity.unit).to_lowercase(),
                        asked_for: record.clamped_from.get(parameter).copied(),
                    })
                    .collect(),
                inputs: record
                    .inputs
                    .iter()
                    .map(|input| InputLine {
                        variable: input.variable.0.clone(),
                        value: input.value.value,
                        unit: format!("{:?}", input.value.unit).to_lowercase(),
                        layer: layer_name(input.layer).to_string(),
                        source: source_name(&input.source),
                    })
                    .collect(),
                note: &record.explanation,
            },
        };

        let line = serde_json::to_string(&request)
            .map_err(|e| LanguageError::Backend(format!("could not encode the record: {e}")))?;
        self.ask(&line)
    }

    fn explain_refusal(&self, asked: &str, reason: &str) -> Result<String, LanguageError> {
        let request = RefusalRequest {
            refusal: Refusal { asked, reason },
        };
        let line = serde_json::to_string(&request)
            .map_err(|e| LanguageError::Backend(format!("could not encode the refusal: {e}")))?;
        self.ask(&line)
    }
}
