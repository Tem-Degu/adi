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
///
/// **Narrow it per machine before handing it to a decoder.** Where an
/// identifier belongs, this says `{"type": "string"}`, because the schema
/// cannot know which styles and variables a given machine declares and the
/// engine validates the answer against the declaration afterwards.
///
/// A decoder can do better than validate afterwards. Given the declared
/// styles as an enum and the declared variables as named properties, a model
/// cannot emit an identifier the machine does not have, rather than emitting
/// one and having it refused. This is not theoretical: against the shipped
/// corpus, an unnarrowed schema produced `adi.espresso.style.cortado` for a
/// machine declaring no such style, while the instruction text had already
/// said to name only what was listed.
///
/// `examples/interpreters/qwen_interpreter.py::schema_for` does this, and is
/// the reference for a backend written in another language.
pub const UNDERSTOOD_SCHEMA: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Understood",
  "description": "What a model may return: a resolved request, a question back, or an answer.",
  "oneOf": [
    { "type": "object", "required": ["intent"], "additionalProperties": false,
      "properties": { "intent": { "$ref": "#/$defs/intent" } } },
    { "type": "object", "required": ["clarify"], "additionalProperties": false,
      "properties": { "clarify": { "$ref": "#/$defs/clarify" } } },
    { "type": "object", "required": ["answer"], "additionalProperties": false,
      "properties": { "answer": { "$ref": "#/$defs/answer" } } }
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
    "answer": {
      "type": "object",
      "required": ["text"],
      "additionalProperties": false,
      "properties": {
        "text": { "type": "string", "minLength": 1 },
        "drew_on": { "type": "array", "items": { "type": "string" } }
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
                "description": "A remembered cycle id, 'last', or 'now' for what the machine is reading at this moment." },
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

State relations rather than resolving them. \"Half of last time\" is
{\"of\": \"last\", \"scale\": 0.5}, never a number. The engine holds the memory
and does the arithmetic.

A relation may also be against what the machine is reading now, which is given
above where the machine reports it. \"A bit warmer\" is
{\"of\": \"now\", \"offset\": {\"value\": 2, \"unit\": \"c\"}}. State the size of the
change you mean; the engine adds it to the reading, clamps the result to what
the machine declares, and refuses it where there is no reading to add to.

Where someone asks for more or less of something without saying how much, that
is the one case to choose a size yourself rather than ask. State it as an
offset against now, keep it modest in proportion to the range the variable is
declared over, and let the engine clamp it. Asking remains correct where the
request is unclear in any other way.

If the person asked a question rather than requesting anything, answer it:
  {\"answer\": {\"text\": \"<what to say>\", \"drew_on\": [\"<note id>\"]}}

Answer only from what is given above. Where it does not cover the question, say
so instead of supplying something plausible. List in drew_on the notes you used.

Name only the styles, materials and variables given above.";

/// A variable as a narrowed schema needs it: its name, and its unit if it has
/// one.
pub struct SchemaVariable<'a> {
    pub name: &'a str,
    pub unit: Option<&'a str>,
}

/// [`UNDERSTOOD_SCHEMA`] narrowed to one machine, ready for a decoder.
///
/// The constant above permits any string where an identifier belongs, because
/// it cannot know what a given machine declares. This closes those holes with
/// what this machine actually has, so a style it does not offer becomes
/// impossible to emit rather than emitted and then refused.
///
/// Built here rather than in a backend. A backend narrowing the schema itself
/// has to hold its own copy of the contract, and a copy is a thing that drifts:
/// the shipped Qwen backend held one with the answer arm missing, so the
/// machine could not answer a question at all, and nothing could detect the
/// difference because the engine never stated the contract on the wire. ADI-P
/// 9.4.1 and 9.4.5 say what the engine must supply; this is the engine saying
/// it.
///
/// Written out rather than produced by resolving `$ref` against the constant.
/// A decoder has to be handed a self-contained schema, and the structure below
/// is the one measured to compile under llama.cpp's grammar converter.
pub fn narrowed_schema(
    styles: &[String],
    materials: &[String],
    variables: &[SchemaVariable<'_>],
) -> serde_json::Value {
    use serde_json::json;

    let units: std::collections::BTreeSet<&str> = variables.iter().filter_map(|v| v.unit).collect();

    // Each branch of a target carries a unit, and a relative target's offset
    // carries its own. Narrowed to the units the declared variables use, so an
    // unmodelled unit is unreachable.
    let unit_schema = if units.is_empty() {
        json!({ "type": "string" })
    } else {
        json!({ "enum": units.iter().collect::<Vec<_>>() })
    };

    let target = json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["value", "unit"],
                "additionalProperties": false,
                "properties": { "value": { "type": "number" }, "unit": unit_schema },
            },
            {
                "type": "object",
                "required": ["of"],
                "additionalProperties": false,
                "properties": {
                    "of": { "type": "string" },
                    "scale": { "type": "number", "minimum": 0 },
                    "offset": {
                        "type": "object",
                        "required": ["value", "unit"],
                        "additionalProperties": false,
                        "properties": { "value": { "type": "number" }, "unit": unit_schema },
                    },
                },
            },
        ]
    });

    let mut intent = serde_json::Map::new();
    intent.insert("kind".into(), json!({ "enum": ["adjust", "produce"] }));

    // A machine declaring no styles cannot be asked for one. An empty enum is
    // unsatisfiable by any grammar, so the key is dropped instead.
    if !styles.is_empty() {
        intent.insert("style".into(), json!({ "enum": styles }));
    }
    if !materials.is_empty() {
        intent.insert("material".into(), json!({ "enum": materials }));
    }

    if !variables.is_empty() {
        // Keyed by name rather than by additionalProperties, which is what
        // makes a grammar enumerate the declared variables and nothing else.
        let properties: serde_json::Map<String, serde_json::Value> = variables
            .iter()
            .map(|v| (v.name.to_string(), target.clone()))
            .collect();
        intent.insert(
            "targets".into(),
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": properties,
            }),
        );
    }

    json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["intent"],
                "additionalProperties": false,
                "properties": {
                    "intent": {
                        "type": "object",
                        "required": ["kind"],
                        "additionalProperties": false,
                        "properties": intent,
                    }
                },
            },
            {
                "type": "object",
                "required": ["clarify"],
                "additionalProperties": false,
                "properties": {
                    "clarify": {
                        "type": "object",
                        "required": ["question", "because"],
                        "additionalProperties": false,
                        "properties": {
                            "question": { "type": "string", "minLength": 1 },
                            "because": { "type": "string" },
                        },
                    }
                },
            },
            {
                "type": "object",
                "required": ["answer"],
                "additionalProperties": false,
                "properties": {
                    "answer": {
                        "type": "object",
                        "required": ["text"],
                        "additionalProperties": false,
                        "properties": {
                            "text": { "type": "string", "minLength": 1 },
                            "drew_on": { "type": "array", "items": { "type": "string" } },
                        },
                    }
                },
            },
        ]
    })
}

/// The shape of a selection, narrowed to the candidates offered and the level.
///
/// `count` is how many candidates the request carries, so the permitted indices
/// are `0..count`. At D2 a value is required; at D1 it is absent, because the
/// engine sizes the correction itself.
///
/// An empty candidate list would give an empty enum, which no grammar can
/// satisfy, so the index falls back to an integer there. The engine refuses an
/// unknown index regardless.
pub fn selection_schema(count: usize, level: &str) -> serde_json::Value {
    use serde_json::json;

    let candidate = if count == 0 {
        json!({ "type": "integer" })
    } else {
        json!({ "enum": (0..count).collect::<Vec<_>>() })
    };

    let mut properties = serde_json::Map::new();
    properties.insert("candidate".into(), candidate);

    let required: Vec<&str> = if level == "d2" {
        properties.insert("value".into(), json!({ "type": "number" }));
        vec!["candidate", "value"]
    } else {
        vec!["candidate"]
    };

    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["steps"],
        "properties": {
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": required,
                    "properties": properties,
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use adi_decision::Understood;
    use adi_protocol::ids::VariableId;
    use adi_protocol::intent::{Intent, IntentKind, Reference, RelativeSpec, TargetSpec};
    use adi_protocol::units::{Quantity, Unit};
    use std::collections::BTreeMap;

    fn round_trip(value: &Understood) -> Understood {
        let json = serde_json::to_string(value).expect("serialises");
        serde_json::from_str(&json).expect("and comes back")
    }

    /// ADI-P 9.4 is normative: a relative target MUST be resolved by the
    /// engine and not by the model, because a model required to emit a number
    /// would have to be trusted to remember correctly and to divide correctly,
    /// and would be wrong silently when it did not.
    ///
    /// The instruction has to say so. This paragraph was in the shipped Qwen
    /// backend's own copy and not in this one, so consolidating the two
    /// removed it, and the corpus went from nine of eighteen to four: the
    /// cases that failed were the relative ones.
    #[test]
    fn the_instruction_says_to_state_a_relation_rather_than_resolve_it() {
        let lower = OUTPUT_INSTRUCTION.to_lowercase();
        assert!(
            lower.contains("state relations rather than resolving them"),
            "a model told nothing about this resolves them itself"
        );
        assert!(
            OUTPUT_INSTRUCTION.contains("\"scale\": 0.5"),
            "with the worked example, which is what made it land"
        );
    }

    /// The arm the shipped backend's own copy was missing. A machine that
    /// cannot emit this cannot answer a question at all, whatever model it
    /// runs, because the decoder forbids the characters.
    #[test]
    fn a_narrowed_schema_still_permits_an_answer() {
        let schema = narrowed_schema(&[], &[], &[]);
        let arms = schema["oneOf"].as_array().expect("arms");

        for arm in ["intent", "clarify", "answer"] {
            assert!(
                arms.iter().any(|a| a["properties"].get(arm).is_some()),
                "narrowing must not drop the {arm} arm: {schema}"
            );
        }
    }

    /// Narrowing is the point: an identifier the machine does not declare
    /// becomes impossible to emit rather than emitted and then refused.
    #[test]
    fn a_declared_style_becomes_the_only_one_namable() {
        let styles = ["adi.example.style.standard".to_string()];
        let schema = narrowed_schema(&styles, &[], &[]);
        let intent = &schema["oneOf"][0]["properties"]["intent"]["properties"];

        assert_eq!(
            intent["style"]["enum"],
            serde_json::json!(["adi.example.style.standard"]),
            "got {intent}"
        );
    }

    /// An empty enum is unsatisfiable by any grammar, so a machine declaring
    /// no styles must not be given an empty list of them to choose from.
    #[test]
    fn a_machine_with_no_styles_is_not_asked_for_one() {
        let schema = narrowed_schema(&[], &[], &[]);
        let intent = &schema["oneOf"][0]["properties"]["intent"]["properties"];

        assert!(intent.get("style").is_none(), "got {intent}");
        assert!(intent.get("targets").is_none(), "got {intent}");
    }

    /// Targets are keyed by declared variable, which is what makes a grammar
    /// enumerate them. An unmodelled variable is then unreachable rather than
    /// refused several steps later.
    #[test]
    fn targets_are_keyed_by_declared_variable_and_unit() {
        let variables = [
            SchemaVariable {
                name: "additive_volume",
                unit: Some("ml"),
            },
            SchemaVariable {
                name: "elapsed",
                unit: Some("s"),
            },
        ];
        let schema = narrowed_schema(&[], &[], &variables);
        let targets = &schema["oneOf"][0]["properties"]["intent"]["properties"]["targets"];

        assert_eq!(targets["additionalProperties"], serde_json::json!(false));
        assert!(
            targets["properties"]["additive_volume"].is_object(),
            "got {targets}"
        );
        assert!(
            targets["properties"]["elapsed"].is_object(),
            "got {targets}"
        );
        assert!(
            targets["properties"].get("undeclared").is_none(),
            "an undeclared variable must not be namable: {targets}"
        );

        let unit = &targets["properties"]["elapsed"]["oneOf"][0]["properties"]["unit"];
        assert_eq!(unit["enum"], serde_json::json!(["ml", "s"]), "got {unit}");
    }

    /// ADI-P 9.3 requirement 3 on the way out: only an offered index.
    #[test]
    fn a_selection_may_name_only_an_offered_candidate() {
        let schema = selection_schema(3, "d2");
        let candidate = &schema["properties"]["steps"]["items"]["properties"]["candidate"];
        assert_eq!(candidate["enum"], serde_json::json!([0, 1, 2]));
    }

    /// A value is the absolute setting at D2 and absent at D1, where the
    /// engine sizes the correction itself.
    #[test]
    fn a_value_belongs_to_d2_alone() {
        let d2 = selection_schema(2, "d2");
        let step = &d2["properties"]["steps"]["items"];
        assert_eq!(step["required"], serde_json::json!(["candidate", "value"]));

        let d1 = selection_schema(2, "d1");
        let step = &d1["properties"]["steps"]["items"];
        assert_eq!(step["required"], serde_json::json!(["candidate"]));
        assert!(step["properties"].get("value").is_none(), "got {step}");
    }

    /// No candidates would give an empty enum, which no grammar can satisfy.
    #[test]
    fn no_candidates_leaves_a_compilable_schema() {
        let schema = selection_schema(0, "d2");
        let candidate = &schema["properties"]["steps"]["items"]["properties"]["candidate"];
        assert_eq!(candidate["type"], "integer", "got {candidate}");
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
        let target = intent
            .get("targets")
            .unwrap()
            .get("additive_volume")
            .unwrap();
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
        for token in [
            "intent", "clarify", "question", "because", "scale", "offset", "unit",
        ] {
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
