//! What a model is permitted to say.
//!
//! ADI-P 9.3 requirement 3: a model's output is confined to a closed schema,
//! validated before use. This is that schema, as JSON Schema, in the form a
//! constrained decoder takes.
//!
//! The schema is `Understood`, not `Intent`, and the difference matters.
//!
//! Constraining a small model costs accuracy. Published measurement puts the
//! effect on sub-4B models at a measurable drop in correctness, because
//! capacity spent on satisfying a format is capacity not spent on the task.
//! The recommended mitigation is to constrain only what must be constrained.
//!
//! Permitting a question as an alternative to an intent is exactly that
//! mitigation. A model that cannot resolve a request is not forced to invent
//! something schema-valid: it can ask, and the question is free text under no
//! constraint at all. ADI-P 9.4.3 permits this for a different reason, that
//! the alternative to asking is guessing, and the two reasons agree.
//!
//! Written out rather than generated. A generated schema would track the Rust
//! type automatically and would be one more dependency and one more thing
//! whose output nobody reads. The tests below keep the two in step by driving
//! real values through the parser.

/// The JSON Schema a model's output must satisfy.
///
/// Hand this to a constrained decoder, or validate against it afterwards, or
/// both. The engine validates regardless of how the output was produced: a
/// decoder that guarantees the shape does not guarantee the contents.
pub const UNDERSTOOD_SCHEMA: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Understood",
  "description": "What a model may return: a resolved request, or a question.",
  "oneOf": [
    { "type": "object", "required": ["intent"], "additionalProperties": false,
      "properties": { "intent": { "$ref": "#/$defs/intent" } } },
    { "type": "object", "required": ["clarify"], "additionalProperties": false,
      "properties": { "clarify": { "$ref": "#/$defs/clarify" } } }
  ],
  "$defs": {
    "intent": {
      "type": "object",
      "required": ["kind"],
      "additionalProperties": false,
      "properties": {
        "kind": { "enum": ["adjust", "produce"] },
        "style": { "type": "string" },
        "material": { "type": "string" },
        "targets": {
          "type": "object",
          "additionalProperties": { "$ref": "#/$defs/target" }
        }
      }
    },
    "clarify": {
      "type": "object",
      "required": ["question", "because"],
      "additionalProperties": false,
      "properties": {
        "question": { "type": "string", "minLength": 1 },
        "because": { "type": "string" }
      }
    },
    "target": {
      "oneOf": [
        { "$ref": "#/$defs/absolute" },
        { "$ref": "#/$defs/relative" }
      ]
    },
    "absolute": {
      "type": "object",
      "required": ["value", "unit"],
      "additionalProperties": false,
      "properties": {
        "value": { "type": "number" },
        "unit": { "$ref": "#/$defs/unit" }
      }
    },
    "relative": {
      "type": "object",
      "required": ["of"],
      "additionalProperties": false,
      "properties": {
        "of": { "type": "string",
                "description": "A remembered cycle id, or 'last'." },
        "scale": { "type": "number", "minimum": 0 },
        "offset": { "$ref": "#/$defs/absolute" }
      }
    },
    "unit": {
      "enum": ["none","g","g_per_s","ml","ml_per_s","c","bar","s","ms",
               "step","kg_f","pct"]
    }
  }
}"##;

/// A one-paragraph statement of the same contract, for a prompt.
///
/// A constrained decoder makes the shape unavoidable, but a model told nothing
/// about what it is producing will fill a valid shape with poor contents. The
/// schema is the floor; this is the instruction.
pub const OUTPUT_INSTRUCTION: &str = "\
Reply with one JSON object and nothing else.

To act on the request:
  {\"intent\": {\"kind\": \"produce\"|\"adjust\", \"style\": \"<id>\",
   \"targets\": {\"<variable>\": <target>}}}

A target is either {\"value\": <number>, \"unit\": \"<unit>\"}
or {\"of\": \"last\"|\"<cycle id>\", \"scale\": <number>, \"offset\": {...}}.
Give at most one of scale and offset. Neither means the same as then.
A scale of 0 means none at all.

If the request cannot be resolved, ask instead:
  {\"clarify\": {\"question\": \"<what to ask>\", \"because\": \"<why>\"}}

Asking is a correct answer. Do not guess a value, a style or a cycle that was
not stated: the result of this request moves a machine.

Name only the styles, materials and variables given above.";

#[cfg(test)]
mod tests {
    use super::*;
    use adi_decision::Understood;
    use adi_protocol::intent::{Intent, IntentKind, Reference, RelativeSpec, TargetSpec};
    use adi_protocol::ids::VariableId;
    use adi_protocol::units::{Quantity, Unit};
    use std::collections::BTreeMap;

    fn round_trip(value: &Understood) -> Understood {
        let json = serde_json::to_string(value).expect("serialises");
        serde_json::from_str(&json).expect("and comes back")
    }

    #[test]
    fn the_schema_is_valid_json() {
        let parsed: serde_json::Value =
            serde_json::from_str(UNDERSTOOD_SCHEMA).expect("the schema itself must parse");
        assert!(parsed.get("oneOf").is_some(), "an intent or a question");
    }

    /// The shape the schema describes is the shape serde produces. These are
    /// two hand-maintained descriptions of one contract, so they are checked
    /// against each other rather than trusted.
    #[test]
    fn an_intent_serialises_as_the_schema_says() {
        let mut targets = BTreeMap::new();
        targets.insert(
            VariableId::from("additive_volume"),
            TargetSpec::Relative(RelativeSpec {
                of: Reference::Last,
                scale: Some(0.5),
                offset: None,
            }),
        );

        let value = Understood::Intent(Intent {
            kind: IntentKind::Produce,
            style: Some("adi.example.style.standard".into()),
            material: None,
            targets,
        });

        let json = serde_json::to_value(&value).expect("serialises");
        let intent = json.get("intent").expect("externally tagged as 'intent'");
        assert_eq!(intent.get("kind").unwrap(), "produce");
        let target = intent.get("targets").unwrap().get("additive_volume").unwrap();
        assert_eq!(target.get("of").unwrap(), "last");
        assert_eq!(target.get("scale").unwrap(), 0.5);
    }

    #[test]
    fn a_question_serialises_as_the_schema_says() {
        let value = Understood::Clarify {
            question: "Which one did you mean?".into(),
            because: "no cycle was identified".into(),
        };

        let json = serde_json::to_value(&value).expect("serialises");
        let clarify = json.get("clarify").expect("externally tagged as 'clarify'");
        assert_eq!(clarify.get("question").unwrap(), "Which one did you mean?");
        assert!(clarify.get("because").is_some(), "recorded, not shown");
    }

    #[test]
    fn every_shape_a_model_may_produce_round_trips() {
        let mut absolute = BTreeMap::new();
        absolute.insert(
            VariableId::from("aux_level"),
            TargetSpec::Absolute(Quantity::new(120.0, Unit::Ml)),
        );

        let mut offset = BTreeMap::new();
        offset.insert(
            VariableId::from("cycle_time"),
            TargetSpec::Relative(RelativeSpec {
                of: Reference::Last,
                scale: None,
                offset: Some(Quantity::new(3.0, Unit::S)),
            }),
        );

        let mut same = BTreeMap::new();
        same.insert(
            VariableId::from("cycle_time"),
            TargetSpec::Relative(RelativeSpec {
                of: Reference::Cycle("c7".into()),
                scale: None,
                offset: None,
            }),
        );

        let cases = [
            Understood::Intent(Intent {
                kind: IntentKind::Produce,
                style: None,
                material: None,
                targets: BTreeMap::new(),
            }),
            Understood::Intent(Intent {
                kind: IntentKind::Produce,
                style: Some("adi.example.style.standard".into()),
                material: Some("adi.example.material.standard".into()),
                targets: absolute,
            }),
            Understood::Intent(Intent {
                kind: IntentKind::Adjust,
                style: None,
                material: None,
                targets: offset,
            }),
            Understood::Intent(Intent {
                kind: IntentKind::Adjust,
                style: None,
                material: None,
                targets: same,
            }),
            Understood::Clarify {
                question: "Half of what?".into(),
                because: "a quantity with no variable".into(),
            },
        ];

        for case in cases {
            let back = round_trip(&case);
            assert_eq!(
                serde_json::to_value(&case).unwrap(),
                serde_json::to_value(&back).unwrap(),
                "a shape the schema permits did not survive"
            );
        }
    }

    /// The instruction and the schema describe the same contract to two
    /// different readers. A change to one that is not made to the other is how
    /// a model is told to produce something the decoder will refuse.
    #[test]
    fn the_instruction_names_what_the_schema_requires() {
        for token in ["intent", "clarify", "question", "because", "scale", "offset", "unit"] {
            assert!(
                OUTPUT_INSTRUCTION.contains(token),
                "the instruction never mentions '{token}'"
            );
            assert!(
                UNDERSTOOD_SCHEMA.contains(token),
                "the schema never mentions '{token}'"
            );
        }
    }

    /// ADI-P 9.4.3 makes asking an ordinary outcome. A model that is not told
    /// so will treat it as failure and guess instead, which is the behaviour
    /// the permission exists to prevent.
    #[test]
    fn the_instruction_says_that_asking_is_allowed() {
        let lower = OUTPUT_INSTRUCTION.to_lowercase();
        assert!(lower.contains("asking is a correct answer"));
        assert!(lower.contains("do not guess"));
    }
}
