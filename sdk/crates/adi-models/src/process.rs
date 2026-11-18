//! A model that is another program.
//!
//! The seam is a trait, so a model written in anything can serve it provided
//! someone writes the Rust to call it. This removes that proviso: the program
//! reads a request on its standard input and writes an answer on its standard
//! output, and needs to know nothing about this SDK.
//!
//! The same pattern drives a real `libadip` build from the engine's tests. It
//! costs little and it is the difference between "you may use our models" and
//! "you may use any model".
//!
//! One line in, one line out, both JSON. Three of the four seams are served
//! this way; the explainer needs no model, because `RecordExplainer` produces
//! text from the decision record.
//!
//! ```text
//! interpreter  -> {"utterance":"half of last time","styles":[...]}
//!              <- {"intent":{"kind":"produce","targets":{...}}}
//!
//! transcriber  -> {"audio":"<base64 of little endian i16>","sample_rate":16000}
//!              <- {"text":"half of last time","confidence":0.91}
//!
//! speaker      -> {"text":"I have made it finer."}
//!              <- {"audio":"<base64 of little endian i16>","sample_rate":22050}
//! ```
//!
//! An interpreter's reply is exactly what `schema::UNDERSTOOD_SCHEMA`
//! describes. A program that cannot resolve a request replies with a question,
//! which is an ordinary outcome rather than a failure.
//!
//! Audio is base64 over a line protocol rather than raw bytes, because a line
//! protocol that can carry an arbitrary byte is not a line protocol. Samples
//! are 16-bit signed, little endian, single channel, which is what every
//! speech model in this space expects and what `Audio` already holds.

use std::sync::Arc;

use adi_decision::{accept_intent, DecisionError, Interpreter, SelectionContext, Understood};
use adi_language::context::{ContextBudget, ModelContext, StyleInfo, VariableInfo};
use serde::Serialize;

use crate::backend::ProcessBackend;
use crate::ModelError;

/// What the interpreter needs to know about the machine and its domain.
///
/// Grouped rather than passed as eight positional arguments, which is what this
/// had grown into. It is also the list of everything a model is told, which is
/// worth being able to read in one place.
#[derive(Debug, Default, Clone)]
pub struct Surroundings {
    /// Styles the knowledge declares, which an intent may name, each with
    /// what it is. A style reaches a model as an identifier, and a request
    /// names one the way a person would.
    pub styles: Vec<StyleInfo>,
    /// Materials the knowledge declares.
    pub materials: Vec<String>,
    /// Variables the profile models, each with its unit and meaning.
    pub variables: Vec<VariableInfo>,
    /// The manufacturer's own material, for answering a question. Reaches no
    /// actuation value.
    pub notes: Vec<adi_protocol::knowledge::Note>,
    /// What this machine is, where the caller knows.
    pub device: Option<String>,
    /// Actions an operator may name, numbered by position. ADI-P 8.2.
    ///
    /// Supplied by the caller because they come from the declaration rather
    /// than from knowledge, and because the engine and the model must number
    /// them identically: the reply is a position in this list.
    pub actions: Vec<adi_language::context::ActionLine>,
}

/// An action as the backend is told about it. Its index is its position.
#[derive(Debug, Serialize)]
struct ActionLineOut<'a> {
    index: usize,
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

/// A style as the backend is told about it.
#[derive(Debug, Serialize)]
struct StyleLine<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,

    /// The recipe's own bands, as ("yield_mass", "90 to 130 g").
    ///
    /// This type is a SECOND definition of a style's wire shape, beside
    /// `StyleInfo`, and adding the bands to that one left them here. They were
    /// filled in by the engine, dropped on the wire, and nothing reported it -
    /// the model carried on being shown a name and a sentence. A field added
    /// to one and not the other is invisible from both ends, which is the same
    /// way an extra reply arm once sat unreachable because the Python copy of
    /// the schema had never heard of it.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    targets: &'a [(String, String)],
}

/// A note as the backend is told about it.
#[derive(Debug, Serialize)]
struct NoteLineOut<'a> {
    id: &'a str,
    text: &'a str,
}

/// A variable as the backend is told about it.
#[derive(Debug, Serialize)]
struct VariableLine<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meaning: Option<&'a str>,
}

/// What the program is sent. A flattened `ModelContext`, so a program in any
/// language can read it without knowing our types.
#[derive(Debug, Serialize)]
struct Request<'a> {
    utterance: &'a str,
    /// The contract the reply must satisfy, stated by the engine rather than
    /// remembered by the program. ADI-P 9.4.1. A backend that holds its own
    /// copy of this has a copy that can drift, and one did: the shipped Qwen
    /// backend's copy was missing the answer arm, so the machine could not
    /// answer a question at all.
    instruction: &'static str,
    /// The same contract as a schema, narrowed to what this machine declares,
    /// in the form a constrained decoder takes.
    schema: &'a serde_json::Value,
    styles: &'a [StyleLine<'a>],
    materials: &'a [String],
    /// Each with its unit and what it means, rather than a bare name. A model
    /// cannot map a request onto a variable it has only been shown the name of.
    variables: &'a [VariableLine<'a>],
    /// What the manufacturer wants the machine able to say. Supplied so the
    /// model can answer a question; it reaches no actuation value.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    notes: &'a [NoteLineOut<'a>],
    /// What this machine is, where the engine was told.
    #[serde(skip_serializing_if = "Option::is_none")]
    machine: Option<&'a str>,
    /// The actions an operator may name, numbered. A reply naming one returns
    /// its `index` and nothing else. Absent where the machine declares none,
    /// in which case the schema offers no arm for one either.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    actions: &'a [ActionLineOut<'a>],
    /// What the appliance is reading now. A request may be stated entirely
    /// against it, and "a bit warmer" has nothing to be warmer than without
    /// it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    measured: Vec<ReadingOut>,
    remembered: Vec<RememberedLine>,
    history: Vec<HistoryLine>,
    /// The exchanges before this one, oldest first. Omitted when there are
    /// none, so a program serving a path with no conversation sees no field
    /// rather than an empty list it has to interpret.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conversation: Vec<TurnLine>,
}

/// One current reading as the backend is told about it.
/// One exchange on the wire.
#[derive(Debug, Serialize)]
struct TurnLine {
    /// What the person said.
    said: String,
    /// Everything the machine said back, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    replies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReadingOut {
    variable: String,
    value: f64,
    unit: String,
}

#[derive(Debug, Serialize)]
struct RememberedLine {
    cycle_id: String,
    style: Option<String>,
    settings: Vec<String>,
    feedback: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ArgumentOut {
    parameter: String,
    value: f64,
    unit: String,
}

#[derive(Debug, Serialize)]
struct InputOut {
    variable: String,
    value: f64,
    unit: String,
    layer: String,
}

/// What the engine decided earlier, in enough detail to be asked about.
///
/// Three fields until this grew: the cycle, the cause and the tool. Which is
/// enough to resolve "half of last time" and nowhere near enough to answer
/// "why 38 millilitres?", because the 38 was not in it.
#[derive(Debug, Serialize)]
struct HistoryLine {
    cycle_id: String,
    cause: String,
    tool: Option<String>,
    /// Which recipe that tool was serving. Omitted where there was none, so a
    /// named action and an adjustment do not carry an empty field.
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    arguments: Vec<ArgumentOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    asked_for: Vec<ArgumentOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    inputs: Vec<InputOut>,
    chose: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    note: String,
}

/// An interpreter that is an external program.
///
/// The child is started once and kept, because starting a process per request
/// would pay model loading on every utterance, and loading is the expensive
/// part of a language model.
pub struct ProcessInterpreter {
    /// Shared, because one program can serve this seam and the chooser both,
    /// and starting it twice costs a second set of a model's private buffers.
    /// See `crate::backend`.
    backend: Arc<ProcessBackend>,
    budget: ContextBudget,
    knowledge_styles: Vec<StyleInfo>,
    knowledge_materials: Vec<String>,
    variables: Vec<VariableInfo>,
    /// Every note the knowledge holds, filtered per utterance.
    notes: Vec<adi_protocol::knowledge::Note>,
    /// What this machine is, where the caller supplied it.
    device: Option<String>,
    actions: Vec<adi_language::context::ActionLine>,
}

impl ProcessInterpreter {
    /// Start the program.
    ///
    /// What the model is told it may name is supplied here rather than read
    /// per request, because it does not change within a session and sending it
    /// every time would be paid on every utterance.
    pub fn start(
        command: &str,
        args: &[String],
        surroundings: Surroundings,
        budget: ContextBudget,
    ) -> Result<Self, ModelError> {
        let backend = ProcessBackend::start(command, args).map_err(ModelError::Backend)?;
        Ok(Self::over(Arc::new(backend), surroundings, budget))
    }

    /// The same, over a program that is already running.
    ///
    /// This is how one process serves this seam and the chooser together: the
    /// caller starts the program once and hands the same `Arc` to both.
    pub fn over(
        backend: Arc<ProcessBackend>,
        surroundings: Surroundings,
        budget: ContextBudget,
    ) -> Self {
        Self {
            backend,
            budget,
            knowledge_styles: surroundings.styles,
            knowledge_materials: surroundings.materials,
            variables: surroundings.variables,
            notes: surroundings.notes,
            device: surroundings.device,
            actions: surroundings.actions,
        }
    }

    fn exchange(&self, request: &Request<'_>) -> Result<adi_decision::Reply, ModelError> {
        let line = serde_json::to_string(request)
            .map_err(|e| ModelError::Backend(format!("could not encode the request: {e}")))?;

        let reply = self.backend.exchange(&line).map_err(ModelError::Backend)?;

        // A program that returns something outside the schema is refused
        // rather than guessed at. This is the boundary 9.3 requirement 3
        // describes, and it holds whether or not the program constrained its
        // own decoding.
        //
        // Parsed tolerantly as to where the JSON begins and ends, because a
        // program wrapping a model will often return it fenced or prefaced,
        // and strictly as to what it contains. See crate::parse.
        crate::parse::reply(&reply)
    }
}

impl Interpreter for ProcessInterpreter {
    /// The decision alone, for the callers that want only that.
    fn interpret(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError> {
        Ok(self.reply(utterance, context)?.understood)
    }

    fn reply(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<adi_decision::Reply, DecisionError> {
        let mut assembled = ModelContext {
            utterance: utterance.to_string(),
            // Explicit, because the literal below ends in `..Default::default()`
            // and a field left out of it is filled with an empty default rather
            // than refused. Adding `recent` to `ModelContext::assemble` was not
            // enough: this interpreter builds its context by hand and never
            // calls that function, so the conversation compiled, shipped, and
            // arrived at the program as `null`.
            recent: {
                // Truncated HERE as well as in `assemble`, because this
                // interpreter never calls that function. The budget's
                // `conversation` is the number sent; the engine holds more.
                // Oldest first, so the END is what matters and `take` from the
                // front would hide what was just said.
                let n = self.budget.conversation;
                let from = context.recent.len().saturating_sub(n);
                if n == 0 {
                    Vec::new()
                } else {
                    context.recent[from..].to_vec()
                }
            },
            styles: self.knowledge_styles.clone(),
            materials: self.knowledge_materials.clone(),
            variables: self.variables.clone(),
            // All of them. See knowledge::Note: there is no relevance filter,
            // and the size of the set is the author's responsibility.
            notes: self
                .notes
                .iter()
                .map(|n| adi_language::context::NoteLine {
                    id: n.id.clone(),
                    text: n.text.trim().to_string(),
                })
                .collect(),
            device: self.device.clone(),
            actions: self.actions.clone(),
            measured: context
                .measured
                .map(|measured| {
                    measured
                        .iter()
                        .map(|(variable, quantity)| adi_language::context::ReadingLine {
                            name: variable.0.clone(),
                            value: quantity.value,
                            unit: quantity.unit.as_str().to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            ..Default::default()
        };

        if let Some(memory) = context.memory {
            assembled.remembered = memory
                .recent(self.budget.remembered_cycles)
                .into_iter()
                .map(|entry| adi_language::context::RememberedSummary {
                    cycle_id: entry.cycle_id.clone(),
                    style: entry.style.clone(),
                    settings: entry
                        .settings
                        .iter()
                        .map(|(v, q)| format!("{}={}{:?}", v.0, q.value, q.unit))
                        .collect(),
                    feedback: entry.feedback.descriptors.clone(),
                })
                .collect();
        }

        assembled.history = context
            .history
            .iter()
            .take(self.budget.history)
            // The shared summariser, not a second copy of it. This was a
            // hand-written DecisionSummary and it carried three of the
            // record's fields while the type had grown to ten.
            .map(adi_language::context::summarise_decision)
            .collect();

        // The budget applies here as it does to an assembled context. Without
        // this the notes went out whole, and a set larger than the model's
        // context overflowed it with nothing saying so.
        assembled.enforce(self.budget);

        // Everything that was dropped, not the first thing.
        //
        // This reported notes alone. `enforce` records every section it
        // removes, in the order it removes them, and the later ones matter
        // more: notes degrade an answer, while styles leave a model unable to
        // name a drink at all. With a budget too small for a full declaration
        // the engine deleted the entire drink list, said "the notes were
        // truncated", and the model then asked what was meant - which reads as
        // the model being slow rather than as the prompt being gutted.
        if !assembled.truncated.is_empty() {
            eprintln!(
                "adi: the prompt did not fit the context budget of {} tokens, so these were dropped: {}.",
                self.budget.max_tokens,
                assembled.truncated.join(", ")
            );

            // Named separately because it is the one that stops the machine
            // working rather than making it less informed.
            if assembled.truncated.contains(&"styles") {
                eprintln!(
                    "     Without styles the model is shown no drinks and cannot name one. Raise `context_tokens` for this seam."
                );
            } else if assembled.truncated.contains(&"notes") {
                eprintln!(
                    "     Notes are what answers a question, so questions will be answered worse. Raise `context_tokens`, or shorten them."
                );
            }
        }

        let style_lines: Vec<StyleLine<'_>> = assembled
            .styles
            .iter()
            .map(|s| StyleLine {
                name: &s.name,
                description: s.description.as_deref(),
                targets: &s.targets,
            })
            .collect();

        let variable_lines: Vec<VariableLine<'_>> = assembled
            .variables
            .iter()
            .map(|v| VariableLine {
                name: &v.name,
                unit: v.unit.as_deref(),
                meaning: v.meaning.as_deref(),
            })
            .collect();

        let action_lines: Vec<ActionLineOut<'_>> = assembled
            .actions
            .iter()
            .enumerate()
            .map(|(index, a)| ActionLineOut {
                index,
                name: &a.name,
                description: a.description.as_deref(),
            })
            .collect();

        let note_lines: Vec<NoteLineOut<'_>> = assembled
            .notes
            .iter()
            .map(|n| NoteLineOut {
                id: &n.id,
                text: &n.text,
            })
            .collect();

        let schema_variables: Vec<crate::SchemaVariable<'_>> = assembled
            .variables
            .iter()
            .map(|v| crate::SchemaVariable {
                name: &v.name,
                unit: v.unit.as_deref(),
            })
            .collect();
        // The schema is narrowed from the identifiers. A description is for the
        // prompt and is not something a model may emit.
        let style_names: Vec<String> = assembled.styles.iter().map(|s| s.name.clone()).collect();
        let schema =
            crate::narrowed_schema(
            &style_names,
            &assembled.materials,
            &schema_variables,
            assembled.actions.len(),
        );

        let request = Request {
            utterance: &assembled.utterance,
            instruction: crate::OUTPUT_INSTRUCTION,
            actions: &action_lines,
            schema: &schema,
            styles: &style_lines,
            materials: &assembled.materials,
            variables: &variable_lines,
            notes: &note_lines,
            machine: assembled.device.as_deref(),
            measured: assembled
                .measured
                .iter()
                .map(|r| ReadingOut {
                    variable: r.name.clone(),
                    value: r.value,
                    unit: r.unit.clone(),
                })
                .collect(),
            remembered: assembled
                .remembered
                .iter()
                .map(|r| RememberedLine {
                    cycle_id: r.cycle_id.clone(),
                    style: r.style.clone(),
                    settings: r.settings.clone(),
                    feedback: r.feedback.clone(),
                })
                .collect(),
            conversation: assembled
                .recent
                .iter()
                .map(|t| TurnLine {
                    said: t.said.clone(),
                    replies: t.replies.clone(),
                })
                .collect(),
            history: assembled
                .history
                .iter()
                .map(|h| HistoryLine {
                    cycle_id: h.cycle_id.clone(),
                    cause: h.cause.clone(),
                    tool: h.tool.clone(),
                    style: h.style.clone(),
                    arguments: h
                        .arguments
                        .iter()
                        .map(|a| ArgumentOut {
                            parameter: a.parameter.clone(),
                            value: a.value,
                            unit: a.unit.clone(),
                        })
                        .collect(),
                    asked_for: h
                        .asked_for
                        .iter()
                        .map(|a| ArgumentOut {
                            parameter: a.parameter.clone(),
                            value: a.value,
                            unit: a.unit.clone(),
                        })
                        .collect(),
                    inputs: h
                        .inputs
                        .iter()
                        .map(|i| InputOut {
                            variable: i.variable.clone(),
                            value: i.value,
                            unit: i.unit.clone(),
                            layer: i.layer.clone(),
                        })
                        .collect(),
                    chose: h.chose,
                    note: h.note.clone(),
                })
                .collect(),
        };

        let answer = self
            .exchange(&request)
            .map_err(|e| DecisionError::Backend(e.to_string()))?;

        // The same validation every interpreter's output passes through. A
        // program is not trusted more than a model linked into this binary.
        //
        // The decision is validated; the sentence is not, because there is
        // nothing to validate it against and it reaches no actuator. What
        // protects it is when the engine chooses to speak it: only once the
        // appliance has accepted.
        Ok(adi_decision::Reply {
            understood: accept_intent(answer.understood)?,
            say: answer.say,
        })
    }
}

// The child is reaped by `ProcessBackend`'s own `Drop`, when the last seam
// holding it releases it. This seam used to kill its own child, which is why
// one program serving two seams needed that lifetime to move to the pipe.
