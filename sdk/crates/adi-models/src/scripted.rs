//! An interpreter whose answers come from a file.
//!
//! Not a mock in a test module. It is a configured backend like any other, and
//! it exists so the whole request path can be exercised and measured before a
//! model is involved at all.
//!
//! Two uses, both real:
//!
//! **Proving the path.** Everything from an utterance to a moving mechanism
//! can be tested with known answers, so when a model is substituted a failure
//! is attributable to the model rather than to the plumbing around it. That
//! distinction is hard to recover if both are new at once.
//!
//! **Measuring the rest.** `adi models bench` reports what a request costs. A
//! scripted backend gives the floor, which is what the engine costs with the
//! model time removed.

use std::collections::BTreeMap;
use std::path::Path;

use adi_decision::{accept_intent, DecisionError, Interpreter, SelectionContext, Understood};
use serde::Deserialize;

use crate::ModelError;

/// One scripted answer.
///
/// The two arms are named rather than flattened into `Understood`. Serde
/// cannot flatten an externally tagged enum, and writing them out also makes
/// the file legible to someone who has not read the Rust.
#[derive(Debug, Clone, Deserialize)]
struct Entry {
    /// The utterance this answers, matched case-insensitively after trimming.
    say: String,
    #[serde(default)]
    intent: Option<adi_protocol::Intent>,
    #[serde(default)]
    clarify: Option<Clarify>,
}

#[derive(Debug, Clone, Deserialize)]
struct Clarify {
    question: String,
    #[serde(default)]
    because: String,
}

impl Entry {
    fn answer(self) -> Result<(String, Understood), String> {
        match (self.intent, self.clarify) {
            (Some(intent), None) => Ok((self.say, Understood::Intent(intent))),
            (None, Some(clarify)) => Ok((
                self.say,
                Understood::Clarify {
                    question: clarify.question,
                    because: clarify.because,
                },
            )),
            (Some(_), Some(_)) => Err(format!(
                "'{}' gives both an intent and a question; an answer is one or the other",
                self.say
            )),
            (None, None) => Err(format!("'{}' gives neither an intent nor a question", self.say)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Script {
    #[serde(default)]
    answers: Vec<Entry>,
}

/// An interpreter that looks its answers up.
pub struct ScriptedInterpreter {
    answers: BTreeMap<String, Understood>,
}

impl ScriptedInterpreter {
    pub fn load(path: &Path) -> Result<Self, ModelError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ModelError::Io(path.display().to_string(), e.to_string()))?;
        let script: Script = serde_yaml::from_str(&text)
            .map_err(|e| ModelError::Malformed(path.display().to_string(), e.to_string()))?;

        let mut answers = BTreeMap::new();
        for entry in script.answers {
            let (say, answer) = entry
                .answer()
                .map_err(|e| ModelError::Malformed(path.display().to_string(), e))?;
            answers.insert(normalise(&say), answer);
        }

        Ok(Self { answers })
    }

    /// Build one directly, for a test that would rather not carry a file.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, Understood)>) -> Self {
        Self {
            answers: pairs
                .into_iter()
                .map(|(say, answer)| (normalise(&say), answer))
                .collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.answers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }
}

fn normalise(utterance: &str) -> String {
    utterance.trim().to_lowercase()
}

impl Interpreter for ScriptedInterpreter {
    fn interpret(
        &self,
        utterance: &str,
        _context: &SelectionContext<'_>,
    ) -> Result<Understood, DecisionError> {
        let answer = self.answers.get(&normalise(utterance)).cloned();

        // An utterance with no scripted answer becomes a question, not an
        // error. That is what a model does when it cannot resolve a request,
        // so the scripted path behaves as the real one does at its edges
        // rather than only in the middle.
        let answer = answer.unwrap_or_else(|| Understood::Clarify {
            question: "Sorry, I did not understand that. Could you say it another way?".into(),
            because: "the scripted backend has no answer for this utterance".into(),
        });

        accept_intent(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adi_protocol::intent::{Intent, IntentKind};

    fn context() -> SelectionContext<'static> {
        SelectionContext {
            history: &[],
            cycle: None,
            memory: None,
            rejected: None,
            settings: None,
            measured: None,
        }
    }

    fn produce(style: &str) -> Understood {
        Understood::Intent(Intent {
            kind: IntentKind::Produce,
            style: Some(style.into()),
            material: None,
            targets: BTreeMap::new(),
        })
    }

    #[test]
    fn a_scripted_answer_is_returned() {
        let interpreter = ScriptedInterpreter::from_pairs([(
            "make one".to_string(),
            produce("adi.example.style.standard"),
        )]);

        match interpreter.interpret("make one", &context()).unwrap() {
            Understood::Intent(intent) => {
                assert_eq!(intent.style.as_deref(), Some("adi.example.style.standard"));
            }
            other => panic!("expected an intent, got {other:?}"),
        }
    }

    #[test]
    fn matching_ignores_case_and_surrounding_space() {
        let interpreter = ScriptedInterpreter::from_pairs([(
            "Make One".to_string(),
            produce("adi.example.style.standard"),
        )]);
        assert!(matches!(
            interpreter.interpret("  make one  ", &context()).unwrap(),
            Understood::Intent(_)
        ));
    }

    /// The edge that matters. An unscripted utterance behaves the way a model
    /// behaves when it cannot resolve one: it asks.
    #[test]
    fn an_unknown_utterance_asks_rather_than_failing() {
        let interpreter = ScriptedInterpreter::from_pairs([]);
        match interpreter.interpret("something nobody scripted", &context()) {
            Ok(Understood::Clarify { question, because }) => {
                assert!(!question.is_empty(), "there is something to ask");
                assert!(because.contains("scripted"), "and a reason for the record");
            }
            other => panic!("expected a question, got {other:?}"),
        }
    }

    /// A scripted answer is validated exactly as a model's would be. The
    /// backend being ours is not a reason to trust it.
    #[test]
    fn a_scripted_answer_is_still_validated() {
        use adi_protocol::ids::VariableId;
        use adi_protocol::intent::{Reference, RelativeSpec, TargetSpec};

        let mut targets = BTreeMap::new();
        targets.insert(
            VariableId::from("additive_volume"),
            // Both a scale and an offset, which ADI-P 9.4 forbids.
            TargetSpec::Relative(RelativeSpec {
                of: Reference::Last,
                scale: Some(0.5),
                offset: Some(adi_protocol::units::Quantity::new(
                    1.0,
                    adi_protocol::units::Unit::G,
                )),
            }),
        );

        let interpreter = ScriptedInterpreter::from_pairs([(
            "bad".to_string(),
            Understood::Intent(Intent {
                kind: IntentKind::Adjust,
                style: None,
                material: None,
                targets,
            }),
        )]);

        assert!(
            interpreter.interpret("bad", &context()).is_err(),
            "an intent the protocol forbids must not pass merely because we wrote it"
        );
    }
}
