//! The shipped example documents must parse and validate.
//!
//! These double as a check that PROTOCOL.md and the types have not drifted
//! apart, which is the failure mode a specification and its implementation
//! always eventually have.

use adi_protocol::{ConformanceLevel, Declaration, SemanticId, Skill};

fn example(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/declarations")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn ex900() -> Declaration {
    serde_yaml::from_str(&example("ex900.yaml")).expect("ex900.yaml should parse")
}

fn ex900_skill() -> Skill {
    serde_yaml::from_str(&example("ex900.skill.yaml")).expect("skill should parse")
}

#[test]
fn example_declaration_parses_and_validates() {
    let declaration = ex900();
    declaration.validate().expect("ex900 should be valid");
    assert_eq!(declaration.conformance_level, ConformanceLevel::Autonomous);
    assert_eq!(declaration.device.model, "EX-900");
}

#[test]
fn example_skill_references_only_declared_tools() {
    let declaration = ex900();
    let skill = ex900_skill();
    for response in &skill.responses {
        assert!(
            declaration.tool(&response.tool).is_some(),
            "skill references undeclared tool {}",
            response.tool
        );
    }
}

#[test]
fn vendor_tool_is_recognised_as_vendor() {
    let declaration = ex900();
    let sweetener = SemanticId::from("x.example.additive.sweetener_pump");
    let tool = declaration.tool(&sweetener).expect("sweetener declared");
    assert!(tool.id.is_vendor());
    assert!(!tool.id.is_registry());
    assert_eq!(tool.id.vendor_namespace(), Some("x.example"));
}

#[test]
fn declaration_without_independent_envelope_is_rejected() {
    let mut declaration = ex900();
    declaration.envelope.independent_of_engine = false;
    assert!(
        declaration.validate().is_err(),
        "an appliance whose safety depends on the engine must not validate"
    );
}

/// The parser must handle a declaration from a domain it has never heard of.
/// This crate defines the wire format, so a fictional domain has to work
/// exactly as well as the real one.
#[test]
fn neutral_fixture_parses_and_validates() {
    let declaration: Declaration =
        serde_yaml::from_str(&example("neutral-machine.yaml")).expect("fixture should parse");
    declaration.validate().expect("fixture should be valid");
    assert_eq!(declaration.domain_profile, "example/v1");
    assert!(declaration.has_stateful_envelope());
    assert_eq!(declaration.envelope.invariants.len(), 1);
    assert_eq!(declaration.envelope.rate_limits.len(), 1);
    assert!(declaration.engine_tools().count() >= 5);
}

#[test]
fn stateful_envelope_is_detected() {
    let declaration = ex900();
    assert!(declaration.has_stateful_envelope());

    let mut bare = ex900();
    bare.envelope.invariants.clear();
    bare.envelope.rate_limits.clear();
    assert!(!bare.has_stateful_envelope());
}

/// ADI-P 4.2: a vendor tool the engine may select must say what it is for.
///
/// This existed only in `adi validate`, so no test could see it, and a shipped
/// example sat in the repository violating it until the first CI run said so.
/// A rule enforced only by the tool that reports it is a rule that holds until
/// someone runs that tool on the right file.
///
/// Both shipped pairs are checked, because the one that was wrong was the one
/// nobody thought to validate by hand.
#[test]
fn no_shipped_example_makes_an_undescribed_vendor_tool_selectable() {
    for (declaration, skill) in [
        ("ex900.yaml", "ex900.skill.yaml"),
        ("neutral-machine.yaml", "neutral-machine.skill.yaml"),
    ] {
        let d: Declaration =
            serde_yaml::from_str(&example(declaration)).expect("declaration parses");
        let s: Skill = serde_yaml::from_str(&example(skill)).expect("skill parses");

        assert!(
            s.undescribed_vendor_tools(&d).is_empty(),
            "{declaration}: {:?} are selectable and say nothing about what they do",
            s.undescribed_vendor_tools(&d)
        );
    }
}

/// And the check must actually fire, or the test above passes for the wrong
/// reason. Removing a description from a vendor tool has to be caught.
#[test]
fn removing_a_vendor_description_is_caught() {
    let mut d: Declaration =
        serde_yaml::from_str(&example("neutral-machine.yaml")).expect("parses");
    let s: Skill =
        serde_yaml::from_str(&example("neutral-machine.skill.yaml")).expect("parses");

    let vendor = d
        .tools
        .iter_mut()
        .find(|t| t.id.0.starts_with("x."))
        .expect("the fixture declares a vendor tool");
    let id = vendor.id.0.clone();
    vendor.description = None;

    assert_eq!(s.undescribed_vendor_tools(&d), vec![id]);
}

/// A registry tool needs no description of its own: the profile supplies it,
/// and requiring one from every manufacturer would be asking them to retype
/// what the identifier already means.
#[test]
fn a_registry_tool_without_a_description_is_not_faulted() {
    let mut d: Declaration = serde_yaml::from_str(&example("ex900.yaml")).expect("parses");
    let s: Skill = serde_yaml::from_str(&example("ex900.skill.yaml")).expect("parses");

    for tool in d.tools.iter_mut().filter(|t| t.id.0.starts_with("adi.")) {
        tool.description = None;
    }

    assert!(s.undescribed_vendor_tools(&d).is_empty());
}
