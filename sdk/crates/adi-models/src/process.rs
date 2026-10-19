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

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use adi_decision::{accept_intent, DecisionError, Interpreter, SelectionContext, Understood};
use adi_language::context::{ContextBudget, ModelContext, VariableInfo};
use serde::Serialize;

use crate::ModelError;

/// What the interpreter needs to know about the machine and its domain.
///
/// Grouped rather than passed as eight positional arguments, which is what this
/// had grown into. It is also the list of everything a model is told, which is
/// worth being able to read in one place.
#[derive(Debug, Default, Clone)]
pub struct Surroundings {
    /// Styles the knowledge declares, which an intent may name.
    pub styles: Vec<String>,
    /// Materials the knowledge declares.
    pub materials: Vec<String>,
    /// Variables the profile models, each with its unit and meaning.
    pub variables: Vec<VariableInfo>,
    /// The manufacturer's own material, for answering a question. Reaches no
    /// actuation value.
    pub notes: Vec<adi_protocol::knowledge::Note>,
    /// What this machine is, where the caller knows.
    pub device: Option<String>,
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
    styles: &'a [String],
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
    remembered: Vec<RememberedLine>,
    history: Vec<HistoryLine>,
}

#[derive(Debug, Serialize)]
struct RememberedLine {
    cycle_id: String,
    style: Option<String>,
    settings: Vec<String>,
    feedback: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HistoryLine {
    cycle_id: String,
    cause: String,
    tool: Option<String>,
}

/// An interpreter that is an external program.
///
/// The child is started once and kept, because starting a process per request
/// would pay model loading on every utterance, and loading is the expensive
/// part of a language model.
pub struct ProcessInterpreter {
    inner: Mutex<Pipe>,
    budget: ContextBudget,
    knowledge_styles: Vec<String>,
    knowledge_materials: Vec<String>,
    variables: Vec<VariableInfo>,
    /// Every note the knowledge holds, filtered per utterance.
    notes: Vec<adi_protocol::knowledge::Note>,
    /// What this machine is, where the caller supplied it.
    device: Option<String>,
}

struct Pipe {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Kept for error messages. A program that starts and immediately exits
    /// reports only that its output closed, which says nothing about which
    /// program. On Windows `python3` is a store shim that does exactly this.
    command: String,
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
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| ModelError::Backend(format!("could not start '{command}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ModelError::Backend("the program has no standard input".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ModelError::Backend("the program has no standard output".into()))?;

        Ok(Self {
            inner: Mutex::new(Pipe {
                child,
                stdin,
                stdout: BufReader::new(stdout),
                command: command.to_string(),
            }),
            budget,
            knowledge_styles: surroundings.styles,
            knowledge_materials: surroundings.materials,
            variables: surroundings.variables,
            notes: surroundings.notes,
            device: surroundings.device,
        })
    }

    fn exchange(&self, request: &Request<'_>) -> Result<Understood, ModelError> {
        let mut line = serde_json::to_string(request)
            .map_err(|e| ModelError::Backend(format!("could not encode the request: {e}")))?;
        line.push('\n');

        let mut pipe = self
            .inner
            .lock()
            .map_err(|_| ModelError::Backend("the backend lock was poisoned".into()))?;

        pipe.stdin
            .write_all(line.as_bytes())
            .and_then(|()| pipe.stdin.flush())
            .map_err(|e| ModelError::Backend(format!("could not reach the program: {e}")))?;

        let mut reply = String::new();
        let read = pipe
            .stdout
            .read_line(&mut reply)
            .map_err(|e| ModelError::Backend(format!("could not read the reply: {e}")))?;

        if read == 0 {
            return Err(ModelError::Backend(format!(
                "'{}' produced no output and exited. Check that the command exists and                  that the script path is correct; on Windows, 'python3' is often a store                  shim that exits immediately, and 'python' is the working command.",
                pipe.command
            )));
        }

        // A program that returns something outside the schema is refused
        // rather than guessed at. This is the boundary 9.3 requirement 3
        // describes, and it holds whether or not the program constrained its
        // own decoding.
        //
        // Parsed tolerantly as to where the JSON begins and ends, because a
        // program wrapping a model will often return it fenced or prefaced,
        // and strictly as to what it contains. See crate::parse.
        crate::parse::understood(&reply)
    }
}

impl Interpreter for ProcessInterpreter {
    fn interpret(
        &self,
        utterance: &str,
        context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError> {
        let mut assembled = ModelContext {
            utterance: utterance.to_string(),
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
            .map(|record| adi_language::context::DecisionSummary {
                cycle_id: record.cycle_id.clone(),
                cause: record.cause.clone(),
                tool: record.selected_tool.as_ref().map(|t| t.0.clone()),
            })
            .collect();

        // The budget applies here as it does to an assembled context. Without
        // this the notes went out whole, and a set larger than the model's
        // context overflowed it with nothing saying so.
        assembled.enforce(self.budget);

        if assembled.truncated.contains(&"notes") {
            eprintln!(
                "adi: the notes exceed the configured context budget and were truncated.                  Reduce them to what the smallest model you target can hold."
            );
        }

        let variable_lines: Vec<VariableLine<'_>> = assembled
            .variables
            .iter()
            .map(|v| VariableLine {
                name: &v.name,
                unit: v.unit.as_deref(),
                meaning: v.meaning.as_deref(),
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
        let schema =
            crate::narrowed_schema(&assembled.styles, &assembled.materials, &schema_variables);

        let request = Request {
            utterance: &assembled.utterance,
            instruction: crate::OUTPUT_INSTRUCTION,
            schema: &schema,
            styles: &assembled.styles,
            materials: &assembled.materials,
            variables: &variable_lines,
            notes: &note_lines,
            machine: assembled.device.as_deref(),
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
            history: assembled
                .history
                .iter()
                .map(|h| HistoryLine {
                    cycle_id: h.cycle_id.clone(),
                    cause: h.cause.clone(),
                    tool: h.tool.clone(),
                })
                .collect(),
        };

        let understood = self
            .exchange(&request)
            .map_err(|e| DecisionError::Backend(e.to_string()))?;

        // The same validation every interpreter's output passes through. A
        // program is not trusted more than a model linked into this binary.
        accept_intent(understood)
    }
}

impl Drop for ProcessInterpreter {
    fn drop(&mut self) {
        if let Ok(mut pipe) = self.inner.lock() {
            let _ = pipe.child.kill();
            let _ = pipe.child.wait();
        }
    }
}
