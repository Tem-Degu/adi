//! Reading what a model actually returned.
//!
//! A constrained decoder guarantees the shape. Nothing else does, and most
//! backends are not constrained decoders: a Python program wrapping a model,
//! a hosted endpoint, or a model prompted rather than constrained will all
//! return something close to the schema surrounded by things that are not.
//!
//! The common cases, all seen in practice:
//!
//! ```text
//! {"intent": {...}}                     clean
//! ```json\n{"intent": {...}}\n```       fenced
//! Here is the result:\n{"intent": ...}  prefaced
//! {"intent": {...}}\n\nThis sets the…   explained afterwards
//! ```
//!
//! Extracting the object is not leniency about the schema. What is extracted is
//! then validated exactly as before, and a model that returns a well formed
//! intent the protocol forbids is still refused. This only decides where the
//! JSON starts and stops.
//!
//! The alternative is refusing an answer that is present but wrapped, which
//! turns a usable backend into an unusable one for a cosmetic reason.

use adi_decision::Understood;

use crate::ModelError;

/// Parse a model's raw output.
pub fn understood(raw: &str) -> Result<Understood, ModelError> {
    let candidate = extract(raw).ok_or_else(|| {
        ModelError::Backend(format!(
            "no JSON object in the reply: {}",
            truncate(raw.trim(), 200)
        ))
    })?;

    serde_json::from_str(candidate).map_err(|e| {
        ModelError::Backend(format!(
            "the reply is not a valid answer: {e}; got {}",
            truncate(candidate, 200)
        ))
    })
}

/// Find the outermost JSON object in a string.
///
/// Brace counting rather than a regular expression, because the object nests
/// and a regular expression cannot match balanced delimiters. Strings are
/// tracked so that a brace inside a question, which is free text a model
/// writes, does not end the object early.
fn extract(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, byte) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }

    // Unbalanced: the model was cut off mid-object, usually by a token limit.
    // Returning the fragment would produce a confusing parse error, so this
    // reports the more useful fact that there was no complete object.
    None
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit).collect();
    format!("{cut}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use adi_protocol::intent::IntentKind;

    const CLEAN: &str = r#"{"intent":{"kind":"produce","style":"adi.example.style.standard"}}"#;

    fn style_of(answer: Understood) -> Option<String> {
        match answer {
            Understood::Intent(intent) => {
                assert_eq!(intent.kind, IntentKind::Produce);
                intent.style
            }
            other => panic!("expected an intent, got {other:?}"),
        }
    }

    #[test]
    fn clean_json_parses() {
        assert_eq!(
            style_of(understood(CLEAN).unwrap()).as_deref(),
            Some("adi.example.style.standard")
        );
    }

    #[test]
    fn a_fenced_block_parses() {
        let raw = format!("```json\n{CLEAN}\n```");
        assert!(style_of(understood(&raw).unwrap()).is_some());
    }

    #[test]
    fn a_fence_without_a_language_parses() {
        let raw = format!("```\n{CLEAN}\n```");
        assert!(style_of(understood(&raw).unwrap()).is_some());
    }

    #[test]
    fn a_preface_is_ignored() {
        let raw = format!("Here is the structured intent:\n\n{CLEAN}");
        assert!(style_of(understood(&raw).unwrap()).is_some());
    }

    #[test]
    fn an_explanation_afterwards_is_ignored() {
        let raw = format!("{CLEAN}\n\nThis produces a standard cycle.");
        assert!(style_of(understood(&raw).unwrap()).is_some());
    }

    #[test]
    fn a_preface_and_an_explanation_together_are_ignored() {
        let raw = format!("Sure! Here you go:\n```json\n{CLEAN}\n```\nHope that helps.");
        assert!(style_of(understood(&raw).unwrap()).is_some());
    }

    #[test]
    fn whitespace_does_not_matter() {
        let raw = format!("\n\n   {CLEAN}   \n\n");
        assert!(style_of(understood(&raw).unwrap()).is_some());
    }

    /// A question is free text a model writes, so it may contain braces. The
    /// object must not end at the first one inside a string.
    #[test]
    fn a_brace_inside_a_question_does_not_end_the_object() {
        let raw = r#"{"clarify":{"question":"Did you mean {a} or {b}?","because":"ambiguous"}}"#;
        match understood(raw).unwrap() {
            Understood::Clarify { question, .. } => {
                assert_eq!(question, "Did you mean {a} or {b}?");
            }
            other => panic!("expected a question, got {other:?}"),
        }
    }

    #[test]
    fn an_escaped_quote_inside_a_question_is_handled() {
        let raw = r#"{"clarify":{"question":"Did you say \"half\"?","because":"unclear"}}"#;
        match understood(raw).unwrap() {
            Understood::Clarify { question, .. } => assert!(question.contains("half")),
            other => panic!("expected a question, got {other:?}"),
        }
    }

    #[test]
    fn nested_objects_survive() {
        let raw = r#"{"intent":{"kind":"adjust","targets":{"cycle_time":{"of":"last","scale":0.5}}}}"#;
        match understood(raw).unwrap() {
            Understood::Intent(intent) => assert_eq!(intent.targets.len(), 1),
            other => panic!("expected an intent, got {other:?}"),
        }
    }

    // ------------------------------------------------------------- refusals

    #[test]
    fn prose_with_no_object_is_refused() {
        let err = understood("I think you want the usual one.").expect_err("no JSON here");
        assert!(format!("{err}").contains("no JSON object"), "{err}");
    }

    /// Cut off by a token limit. Reporting that there is no complete object is
    /// more useful than a parse error about an unexpected end of input.
    #[test]
    fn a_truncated_object_is_refused() {
        let err = understood(r#"{"intent":{"kind":"produce""#).expect_err("incomplete");
        assert!(format!("{err}").contains("no JSON object"), "{err}");
    }

    #[test]
    fn a_complete_object_that_is_not_an_answer_is_refused() {
        let err = understood(r#"{"something":"else"}"#).expect_err("wrong shape");
        assert!(format!("{err}").contains("not a valid answer"), "{err}");
    }

    /// Extraction decides where the JSON is. It does not decide whether the
    /// contents are permitted, and must not become a way to smuggle something
    /// past the schema.
    #[test]
    fn finding_the_json_does_not_loosen_the_schema() {
        let raw = r#"Here: {"intent":{"kind":"demolish"}}"#;
        assert!(understood(raw).is_err(), "an undefined kind must still fail");
    }

    #[test]
    fn a_very_long_reply_is_truncated_in_the_error() {
        let long = "x".repeat(5000);
        let err = understood(&long).expect_err("no JSON");
        assert!(
            format!("{err}").len() < 400,
            "an error message should not quote five thousand characters"
        );
    }
}
