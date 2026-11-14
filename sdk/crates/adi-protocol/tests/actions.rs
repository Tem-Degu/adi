//! The actions an operator may name. ADI-P 8.2.
//!
//! The test that matters here is `a_tool_needing_a_value_is_never_an_action`.
//! Everything else in this mechanism rests on it: if a tool that requires a
//! magnitude can reach this list, then a model choosing from the list has
//! chosen a magnitude, and ADI-P 9.3 is broken. It is a test rather than a
//! comment for that reason.

use adi_protocol::Declaration;

fn declaration(tools: &str) -> Declaration {
    let yaml = format!(
        r#"
adip_version: "0.2"
domain: "espresso"
domain_profile: "espresso/v1"
conformance_level: parameterised
device:
  vendor: "Test"
  model: "Test"
  firmware_version: "1.0.0"
  declaration_id: "sha256:test"
session:
  max_concurrent: 1
  heartbeat_interval_ms: 500
  control_timeout_ms: 2000
tools:
{tools}
envelope:
  enforced_by: firmware
  independent_of_engine: true
  failsafe:
    on_control_timeout: "abort_to_safe"
    on_signal_loss: "abort_to_safe"
    on_fault: "abort_to_safe"
    safe_state: "idle"
    max_time_to_safe_ms: 100
"#
    );
    serde_yaml::from_str(&yaml).expect("the test declaration should parse")
}

/// The safety property of the whole mechanism.
///
/// A tool with a required parameter is a tool that needs a number, and nobody
/// on this path supplies one. If such a tool appeared here, naming it would
/// invoke it with the argument missing - which the appliance refuses - or
/// would require the arm to carry a value, which is precisely what ADI-P 9.3
/// requirement 3 forbids a model to do.
#[test]
fn a_tool_needing_a_value_is_never_an_action() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.brew.stop"
    name: "stop"
    parameters: {}
    effects: ["yield_mass"]
    reversible: false
  - id: "adi.espresso.brew.start"
    name: "brew"
    parameters:
      volume: { type: number, unit: ml, min: 25, max: 240 }
    effects: ["yield_mass"]
    reversible: false
"#,
    );

    let actions = declaration.operator_actions();

    assert_eq!(actions.len(), 1, "only the parameterless tool may be named");
    assert_eq!(actions[0].tool.as_str(), "adi.espresso.brew.stop");
}

/// Optional, not merely defaulted.
///
/// `adip_validate.c` checks `ADIP_PARAM_F_OPTIONAL` and nothing else, so a
/// defaulted-but-required argument left out of an invocation is refused, and
/// reported as a precondition failure at that. Reading `default` here would
/// build a list of actions the appliance rejects, and the diagnostic would
/// send whoever chased it to the preconditions.
#[test]
fn a_default_does_not_make_a_parameter_optional() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.additive.dispense"
    name: "dispense"
    parameters:
      substance: { type: enum, values: ["syrup"], default: "syrup" }
    effects: ["cup_contents"]
    reversible: false
"#,
    );

    assert!(
        declaration.operator_actions().is_empty(),
        "a defaulted parameter is still required, so this tool is not namable"
    );
}

/// Where every parameter really is optional the tool qualifies, and the values
/// used are the manufacturer's own. Nothing a model said reaches the machine.
#[test]
fn every_parameter_optional_is_namable() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.maintenance.flush"
    name: "rinse"
    parameters:
      volume: { type: number, unit: ml, min: 0, max: 100, default: 50, optional: true }
    effects: ["cup_contents"]
    reversible: false
"#,
    );

    let actions = declaration.operator_actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].name, "rinse");
}

#[test]
fn a_tool_the_operator_may_not_touch_is_excluded() {
    let declaration = declaration(
        r#"
  - id: "x.test.internal.calibrate"
    name: "calibrate"
    parameters: {}
    effects: ["cup_contents"]
    reversible: true
    authority: ["engine"]
"#,
    );

    assert!(declaration.operator_actions().is_empty());
}

/// The index is a position in this list, not the tool's position in the
/// declaration. A model is given the list and answers against it, so the two
/// must not be confused: `bob_sim.c` has the same trap for tool indices, and
/// getting it wrong there drives the wrong mechanism while compiling cleanly.
#[test]
fn indices_number_the_list_and_not_the_declaration() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.brew.start"
    name: "brew"
    parameters:
      volume: { type: number, unit: ml, min: 25, max: 240 }
    effects: ["yield_mass"]
    reversible: false
  - id: "adi.espresso.brew.stop"
    name: "stop"
    parameters: {}
    effects: ["yield_mass"]
    reversible: false
  - id: "x.test.cup.clear"
    name: "new_cup"
    parameters: {}
    effects: ["cup_contents"]
    reversible: false
"#,
    );

    let actions = declaration.operator_actions();

    assert_eq!(actions.len(), 2);
    // brew.stop is the second tool declared and the first namable action.
    assert_eq!(actions[0].index, 0);
    assert_eq!(actions[0].tool.as_str(), "adi.espresso.brew.stop");
    assert_eq!(actions[1].index, 1);
    assert_eq!(actions[1].tool.as_str(), "x.test.cup.clear");
}

#[test]
fn a_tool_without_a_name_falls_back_to_its_identifier() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.brew.stop"
    parameters: {}
    effects: ["yield_mass"]
    reversible: false
"#,
    );

    assert_eq!(declaration.operator_actions()[0].name, "stop");
}

/// 4.2 says a hazard informs confirmation policy, so the list has to carry it.
/// A rinse pushes hot water through the group, and a word misheard in a noisy
/// room should not start that without someone being asked first.
#[test]
fn a_hazard_is_carried_so_a_caller_can_confirm() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.maintenance.flush"
    name: "rinse"
    parameters: {}
    effects: ["cup_contents"]
    reversible: false
    hazard: ["hot_liquid"]
  - id: "x.test.cup.clear"
    name: "new_cup"
    parameters: {}
    effects: ["cup_contents"]
    reversible: false
"#,
    );

    let actions = declaration.operator_actions();
    assert_eq!(actions[0].hazard, ["hot_liquid"]);
    assert!(
        actions[1].hazard.is_empty(),
        "declaring a fresh cup drives nothing and needs no confirmation"
    );
}

/// A vendor action with no description is refused.
///
/// `Skill::undescribed_vendor_tools` cannot catch this: it is scoped to tools a
/// skill makes selectable, and a namable action has no response by definition.
/// So the rule 4.2 states for the one case had a hole in it exactly where the
/// other case lives.
#[test]
fn a_vendor_action_with_no_description_is_refused() {
    let declaration = declaration(
        r#"
  - id: "x.test.aux.purge_three"
    name: "purge_three"
    parameters: {}
    effects: ["cup_contents"]
    reversible: false
"#,
    );

    assert_eq!(
        declaration.undescribed_named_actions(),
        vec!["x.test.aux.purge_three".to_string()],
        "a bare vendor identifier in a numbered list is a guess"
    );
}

#[test]
fn a_described_vendor_action_is_accepted() {
    let declaration = declaration(
        r#"
  - id: "x.test.aux.purge_three"
    name: "purge_three"
    description: "Clears the third manifold, which holds material between runs."
    parameters: {}
    effects: ["cup_contents"]
    reversible: false
"#,
    );

    assert!(declaration.undescribed_named_actions().is_empty());
}

/// A registry identifier means the same on every machine and the profile
/// supplies its text, so the declaration need not repeat it. 4.2.
#[test]
fn a_registry_action_needs_no_description_of_its_own() {
    let declaration = declaration(
        r#"
  - id: "adi.espresso.brew.stop"
    name: "stop"
    parameters: {}
    effects: ["yield_mass"]
    reversible: false
"#,
    );

    assert!(
        declaration.undescribed_named_actions().is_empty(),
        "the profile describes a registry tool"
    );
}

/// A hazard alone cannot tell a drink from a cleaning cycle.
///
/// Every drink an espresso machine makes involves hot liquid, so a caller that
/// confirms on `hazard` alone asks before every coffee - and a question asked
/// every time is a question that stops being read. What separates them is
/// whether anything arrives: an action with effects is the thing that was
/// asked for, one without them is a routine.
///
/// The asymmetry is the argument. A drink nobody wanted costs a cup; a descale
/// nobody wanted costs twenty minutes and a consumable, and cannot be stopped.
#[test]
fn an_action_says_whether_anything_arrives() {
    let declaration = declaration(
        r#"
        - id: "x.test.drink.cappuccino"
          name: "cappuccino"
          description: "A coffee with milk in it."
          authority: ["operator", "engine"]
          parameters: {}
          effects: ["yield_mass", "milk_volume"]
          hazard: ["hot_liquid", "steam"]
        - id: "x.test.maintenance.descale"
          name: "descale"
          description: "Twenty minutes and a tablet."
          authority: ["operator", "engine"]
          parameters: {}
          effects: []
          hazard: ["hot_liquid", "chemical"]
        "#,
    );

    let actions = declaration.operator_actions();
    assert_eq!(actions.len(), 2);

    // Both are hazardous, so a rule built on hazard alone treats them alike.
    assert!(!actions[0].hazard.is_empty());
    assert!(!actions[1].hazard.is_empty());

    // And this is what tells them apart.
    assert!(
        !actions[0].effects.is_empty(),
        "a drink delivers something, so it needs no confirming"
    );
    assert!(
        actions[1].effects.is_empty(),
        "a routine delivers nothing, which is what makes it worth a question"
    );
}
