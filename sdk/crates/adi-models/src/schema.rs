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

A number that was said is a number. \"Thirty seconds\" is
{\"value\": 30, \"unit\": \"s\"}, and \"down to twelve percent\" is
{\"value\": 12, \"unit\": \"pct\"}. Do not turn it into a relation.

State relations rather than resolving them, where a relation is what was said.
\"Half of last time\" is {\"of\": \"last\", \"scale\": 0.5}, never a number. The
engine holds the memory and does the arithmetic.

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

Where the person asked for one of the machine's own listed actions rather than
for an outcome, name it by its number:
  {\"action\": <number from the actions listed above>}

Those are the things the machine does that are not quantities: stopping what is
pouring, declaring a fresh container, running a rinse. They take no value, so
give none; there is no field for one. Use this only where an action listed above
is what was asked for, and prefer an intent wherever the request is about how
much of something.

Name only the styles, materials and variables given above.

Where you return an intent or an action, you MAY add a short sentence to say
out loud while it is carried out:

  {\"action\": 3, \"say\": \"Cappuccino, coming right up.\"}

Acknowledge what was asked for, in one short friendly sentence. Do NOT state
any quantity, setting or result: you do not know what the machine will choose,
and this is spoken before anything has happened. Do not promise that it
worked. Leave it out entirely if nothing needs saying.";

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
    actions: usize,
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

    let mut schema = json!({
        "oneOf": [
            {
                "type": "object",
                "required": ["intent"],
                "additionalProperties": false,
                "properties": {
                    // Beside the intent, never inside it. `Intent` is a
                    // governed type and free text has no business in one.
                    "say": { "type": "string" },
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
    });

    // The fourth arm, for a declared action an operator named. ADI-P 8.2.
    //
    // An index into the list the engine supplied, and nothing else: 9.3
    // requirement 3 forbids a model naming a tool by string, and this obeys it
    // exactly as `selection_schema` does. There is no sibling field, so there
    // is nowhere to put a magnitude even by accident.
    //
    // Appended rather than inserted, so the position of every existing arm is
    // unchanged for anything reading the schema by index.
    //
    // Omitted entirely where the machine declares no namable action. An empty
    // enum is satisfiable by no value at all, which no grammar can encode, and
    // an arm a model can never legally fill is worse than no arm.
    if actions > 0 {
        schema["oneOf"]
            .as_array_mut()
            .expect("the schema is built here with oneOf as an array")
            .push(json!({
                "type": "object",
                "required": ["action"],
                "additionalProperties": false,
                "properties": {
                    "action": { "enum": (0..actions).collect::<Vec<_>>() },
                    // Beside the index, never inside it. The index stays the
                    // only thing that decides anything; this is what the
                    // machine says while it does it.
                    "say": { "type": "string" }
                },
            }));

        // And a sequence of them, in the order they should happen.
        //
        // "Rinse the milk and then make a cappuccino" is two of the machine's
        // own products in one breath, and the arm above has one slot. Each
        // entry is still a position in the list the engine derived, so nothing
        // opens up: ADI-P 9.3 requirement 3 is about a model not naming a tool
        // by string, and an array of indices names none.
        schema
            .get_mut("oneOf")
            .and_then(|one_of| one_of.as_array_mut())
            .expect("the schema is built here with oneOf as an array")
            .push(json!({
                "type": "object",
                "required": ["actions"],
                "additionalProperties": false,
                "properties": {
                    "actions": {
                        "type": "array",
                        // Two, because one is the arm above. Offering an array
                        // of one as well would give a model two ways to say
                        // the same thing and no reason to prefer either.
                        "minItems": 2,
                        "items": { "enum": (0..actions).collect::<Vec<_>>() }
                    },
                    "say": { "type": "string" }
                },
            }));
    }

    schema
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

    /// One number on the wire, and no room for anything else.
    #[test]
    fn a_sequence_of_actions_is_a_list_of_indices() {
        let json = serde_json::to_string(&Understood::Actions(vec![7, 0]))
            .expect("serialises");
        assert_eq!(json, r#"{"actions":[7,0]}"#);
        assert!(matches!(
            round_trip(&Understood::Actions(vec![7, 0])),
            Understood::Actions(ref a) if a == &[7, 0]
        ));
    }

    #[test]
    fn the_sequence_arm_takes_two_or_more_offered_indices() {
        let schema = narrowed_schema(&[], &[], &[], 3);
        let arm = schema["oneOf"]
            .as_array()
            .expect("oneOf")
            .iter()
            .find(|arm| arm["required"] == serde_json::json!(["actions"]))
            .expect("a sequence arm is offered");

        // Two, because one index is the single-action arm. An array of one
        // would be a second way to say the same thing.
        assert_eq!(arm["properties"]["actions"]["minItems"], 2);

        // Still indices into the engine's own list, which is what ADI-P 9.3
        // requirement 3 is about: a model names no tool by string here.
        assert_eq!(
            arm["properties"]["actions"]["items"]["enum"],
            serde_json::json!([0, 1, 2])
        );

        let mut fields: Vec<&str> = arm["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            ["actions", "say"],
            "a sequence and a sentence, and nothing else to put a value in"
        );
    }

    #[test]
    fn a_named_action_is_a_bare_index_on_the_wire() {
        let json = serde_json::to_string(&Understood::Action(2)).expect("serialises");
        assert_eq!(json, r#"{"action":2}"#);
        assert!(matches!(round_trip(&Understood::Action(2)), Understood::Action(2)));
    }

    /// ADI-P 9.3 requirement 3. The arm carries an index, and words that are
    /// only ever spoken - so a model has nowhere to put a magnitude even if it
    /// produced one, and cannot name a tool the engine did not offer.
    ///
    /// This asserted ONE property until `say` was added. The number was never
    /// the point: what requirement 3 asks is that nothing in the arm can reach
    /// an actuator, and a string that is spoken and discarded cannot. So the
    /// test now names both fields exactly, which is a tighter check than
    /// counting them - a count of two would pass if someone added `volume`.
    #[test]
    fn the_action_arm_permits_an_offered_index_and_a_sentence() {
        let schema = narrowed_schema(&[], &[], &[], 3);
        // Found by what it requires, not by where it sits. This read `.last()`
        // and broke the moment a second action arm was appended after it -
        // a test of the arm order rather than of the arm.
        let arm = schema["oneOf"]
            .as_array()
            .expect("arms")
            .iter()
            .find(|arm| arm["required"] == serde_json::json!(["action"]))
            .expect("the single-action arm is offered");

        assert_eq!(arm["properties"]["action"]["enum"], serde_json::json!([0, 1, 2]));
        assert_eq!(arm["additionalProperties"], serde_json::json!(false));

        // By name, not by count. These two and nothing else, ever.
        let mut permitted: Vec<&String> = arm["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .collect();
        permitted.sort();
        assert_eq!(
            permitted,
            vec!["action", "say"],
            "the arm carries an index and a sentence; anything else could reach an actuator"
        );

        // And the sentence is a string, so it cannot be a number wearing a
        // different name.
        assert_eq!(arm["properties"]["say"]["type"], serde_json::json!("string"));
    }

    /// The sentence is a sibling of the decision, never inside it.
    ///
    /// `Understood` is what the engine acts on, and `say` is not part of it.
    /// Keeping them apart is what makes the string safe: it is parsed by a
    /// wrapper, spoken, and dropped, and the governed types never see it.
    #[test]
    fn what_is_spoken_is_not_part_of_the_decision() {
        let json = serde_json::to_string(&Understood::Action(2)).expect("serialises");
        assert_eq!(
            json, r#"{"action":2}"#,
            "the decision serialises without a trace of the words"
        );

        // A reply carrying both parses into a decision that is still just an
        // index, with the sentence beside it.
        let reply = crate::parse::reply(r#"{"action": 2, "say": "Coming right up."}"#)
            .expect("a decision with a sentence beside it is valid");
        assert!(matches!(reply.understood, Understood::Action(2)));
        assert_eq!(reply.say.as_deref(), Some("Coming right up."));

        // And a blank sentence is no sentence, rather than a pause.
        let quiet = crate::parse::reply(r#"{"action": 1, "say": "   "}"#).expect("valid");
        assert_eq!(quiet.say, None);
    }

    /// Appended, so nothing reading an earlier arm by position moves.
    #[test]
    fn adding_the_action_arm_does_not_move_the_others() {
        let with = narrowed_schema(&[], &[], &[], 2);
        let without = narrowed_schema(&[], &[], &[], 0);
        let with = with["oneOf"].as_array().expect("arms");
        let without = without["oneOf"].as_array().expect("arms");

        // Two arms are added for a machine with namable actions: one index,
        // and a sequence of them.
        assert_eq!(without.len(), 3);
        assert_eq!(with.len(), 5);
        assert_eq!(
            &with[..3],
            &without[..],
            "the arms that were there are untouched and in the same places"
        );
    }

    /// A machine declaring no namable action gets no arm for one.
    ///
    /// An empty enum is satisfiable by nothing at all, which no grammar can
    /// encode, and an arm a model may never legally fill is worse than its
    /// absence. `selection_schema` has the same problem and falls back to a
    /// plain integer; here the honest answer is that the arm does not apply.
    #[test]
    fn a_machine_with_no_named_actions_is_offered_no_arm() {
        let schema = narrowed_schema(&[], &[], &[], 0);
        let arms = schema["oneOf"].as_array().expect("arms");

        assert_eq!(arms.len(), 3);
        for arm in arms {
            assert!(
                arm["properties"].get("action").is_none(),
                "no arm should offer an action here"
            );
        }
    }

    /// The instruction has to describe the arm, or a model constrained to a
    /// schema it was never told about answers with a clarification instead.
    #[test]
    fn the_instruction_describes_a_named_action() {
        assert!(
            OUTPUT_INSTRUCTION.contains(r#"{"action": <number from the actions listed above>}"#),
            "the instruction must show the shape of a named action"
        );
        assert!(
            OUTPUT_INSTRUCTION.to_lowercase().contains("take no value"),
            "and must say it carries no value"
        );
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
        let schema = narrowed_schema(&[], &[], &[], 0);
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
        let schema = narrowed_schema(&styles, &[], &[], 0);
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
        let schema = narrowed_schema(&[], &[], &[], 0);
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
        let schema = narrowed_schema(&[], &[], &variables, 0);
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
