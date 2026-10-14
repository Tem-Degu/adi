//! Knowledge base and extension shapes.
//!
//! The gate for this step: a base file and a vendor extension both parse, and
//! both collapse to the same layer type, so resolution has one kind of thing to
//! combine rather than a special case per source.
//!
//! Fixtures are a fictional domain. This crate defines the format; if it could
//! only be exercised against espresso, "domain-neutral" would be a claim rather
//! than a property. The espresso base and the example from PROTOCOL.md 6.2 are
//! tested in `adi-domain-espresso`, where that vocabulary belongs.

use adi_protocol::knowledge::{ExtensionError, KnowledgeError, VariableDef};
use adi_protocol::ids::VariableId;
use adi_protocol::Knowledge;
use adi_protocol::{KnowledgeBase, KnowledgeExtension, Layer, Range, Unit};

fn load<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/knowledge")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_yaml::from_str(&text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn base() -> KnowledgeBase {
    load("neutral-base.yaml")
}

fn extension() -> KnowledgeExtension {
    load("neutral-extension.yaml")
}

// ------------------------------------------------------------------ shapes

#[test]
fn a_base_parses() {
    let b = base();
    assert_eq!(b.profile, "example/v1");
    assert_eq!(b.styles.len(), 2);
    assert_eq!(b.material_priors.len(), 2);
    assert!(!b.constraints.is_empty(), "the base owns hard constraints");
    assert!(
        !b.preference_variables.is_empty(),
        "the base owns the preference variable list"
    );

    // Default steps carry units, like every other quantity in the protocol.
    let step = b.default_steps.get("cycle_time").expect("a default step");
    assert_eq!(step.unit, Unit::S);
    assert_eq!(step.value, 2.0);
}

#[test]
fn an_extension_parses() {
    let e = extension();
    assert_eq!(e.namespace, "x.acme");
    assert_eq!(e.styles.len(), 1);
    assert_eq!(e.material_priors.len(), 1);
    assert_eq!(e.defaults.len(), 1);
    assert_eq!(e.constraints.len(), 1, "an extension may narrow a constraint");
}

/// The gate for step 1. Two documents of different shape, one thing to resolve.
#[test]
fn both_collapse_to_the_same_layer_type() {
    let b = base().into_layer();
    let v = extension().into_layer(Layer::Vendor);

    assert_eq!(b.layer, Layer::Base);
    assert_eq!(v.layer, Layer::Vendor);
    assert!(b.namespace.is_none(), "the base owns the bare namespace");
    assert_eq!(v.namespace.as_deref(), Some("x.acme"));

    // The extension's `defaults` list became the same map the base uses, so
    // resolution never has to know which shape it came from.
    assert_eq!(v.default_steps.get("input_mass").map(|q| q.value), Some(9.0));
    assert_eq!(
        v.default_steps.get("input_mass").map(|q| q.unit),
        Some(Unit::G)
    );
    assert_eq!(b.default_steps.get("cycle_time").map(|q| q.value), Some(2.0));
}

/// Only the base may say which variables express preference. A vendor able to
/// add one could reclassify a process variable as taste and put it beyond the
/// engine's reach.
#[test]
fn an_extension_cannot_declare_preference_variables() {
    assert!(extension().into_layer(Layer::Vendor).preference_variables.is_empty());
    assert!(!base().into_layer().preference_variables.is_empty());
}

// -------------------------------------------------------------- validation

#[test]
fn a_well_formed_extension_validates() {
    extension()
        .validate(&base())
        .expect("the shipped example must validate against the base it extends");
}

#[test]
fn an_unnamespaced_identifier_is_refused() {
    let mut e = extension();
    e.styles[0].id = "custom".to_string();
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::NotNamespaced { .. })
    ));
}

/// Redefinition is checked before namespacing, because it is the more specific
/// complaint and the more useful message.
#[test]
fn redefining_a_base_identifier_is_refused() {
    let mut e = extension();
    e.styles[0].id = "adi.example.style.standard".to_string();
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::Redefinition(_))
    ));
}

#[test]
fn inheriting_from_nothing_is_refused() {
    let mut e = extension();
    e.styles[0].inherits = Some("adi.example.style.nonexistent".to_string());
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::UnknownParent { .. })
    ));
}

/// A cycle must be reported rather than overflowing the stack, so the walk is
/// iterative with a budget.
#[test]
fn an_inheritance_cycle_is_refused() {
    let mut e = extension();
    let first = e.styles[0].clone();
    let mut second = first.clone();
    second.id = "x.acme.style.other".to_string();
    second.inherits = Some(first.id.clone());
    e.styles[0].inherits = Some(second.id.clone());
    e.styles.push(second);

    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::InheritanceCycle(_))
    ));
}

/// An empty range makes every value illegal, which is never what anyone meant
/// to write.
#[test]
fn an_empty_range_is_refused() {
    let mut e = extension();
    e.styles[0].targets.insert(
        "cycle_time".to_string(),
        Range {
            min: 10.0,
            max: 1.0,
            resolution: None,
            unit: Unit::S,
        },
    );
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::EmptyRange(_))
    ));
}

#[test]
fn extending_the_wrong_profile_is_refused() {
    let mut e = extension();
    e.extends.domain_profile = "somewhere/v9".to_string();
    assert!(matches!(
        e.validate(&base()),
        Err(ExtensionError::WrongProfile { .. })
    ));
}

/// Inheriting from a base style is the normal case and must keep working, since
/// it is how a vendor starts from something rather than restating it.
#[test]
fn inheriting_from_the_base_is_allowed() {
    let e = extension();
    assert_eq!(
        e.styles[0].inherits.as_deref(),
        Some("adi.example.style.standard")
    );
    e.validate(&base()).expect("inheriting from the base is the point");
}

// --------------------------------------------- variables declared as data

/// A style targeting a variable no layer declares is refused.
///
/// Before variables were declared there was nothing to check against, so a typo
/// in a variable name produced a target that silently matched nothing.
#[test]
fn a_style_targeting_an_undeclared_variable_is_refused() {
    let mut base = base();
    base.styles[0]
        .targets
        .insert("cylce_time".into(), Range { min: 1.0, max: 2.0, unit: Unit::S, resolution: None });

    let err = Knowledge::new(base.into_layer()).expect_err("a typo must be caught");
    assert!(
        matches!(err, KnowledgeError::UndeclaredVariable { ref variable, .. } if variable == "cylce_time"),
        "{err:?}"
    );
}

/// A target stated in the wrong unit is refused.
///
/// This is the worse of the two. A range in the wrong unit still compares
/// against a measurement, and quietly decides a cycle was within target.
#[test]
fn a_target_in_the_wrong_unit_is_refused() {
    let mut base = base();
    let target = base.styles[0].targets.get_mut("cycle_time").unwrap();
    target.unit = Unit::G;

    let err = Knowledge::new(base.into_layer()).expect_err("a unit mismatch must be caught");
    assert!(matches!(err, KnowledgeError::TargetUnitMismatch { .. }), "{err:?}");
}

/// PROTOCOL.md 6.3: a vendor may introduce variables in its own namespace.
///
/// This is what makes a genuinely new capability reachable. A tool moving
/// nothing the base models can be declared, and without a variable to name in
/// the skill the engine could never select it.
#[test]
fn a_vendor_may_declare_its_own_variable() {
    let base = base();
    let mut knowledge = Knowledge::new(base.into_layer()).unwrap();

    let mut extension = extension();
    extension.variables.insert(
        VariableId::from("x.acme.tray_state"),
        VariableDef {
            unit: Unit::None,
            description: "How clean the drip tray is.".into(),
        },
    );
    extension.styles.clear();

    knowledge
        .add(extension.into_layer(Layer::Vendor))
        .expect("a namespaced variable is permitted");
}

/// A vendor style may target a variable that vendor declared.
#[test]
fn a_vendor_style_may_target_a_vendor_variable() {
    let base = base();
    let mut knowledge = Knowledge::new(base.into_layer()).unwrap();

    let mut extension = extension();
    extension.variables.insert(
        VariableId::from("x.acme.tray_state"),
        VariableDef {
            unit: Unit::None,
            description: "How clean the drip tray is.".into(),
        },
    );
    extension.styles[0].targets.insert(
        "x.acme.tray_state".into(),
        Range { min: 0.0, max: 1.0, unit: Unit::None, resolution: None },
    );

    knowledge
        .add(extension.into_layer(Layer::Vendor))
        .expect("a vendor may target its own variable");
}

/// An envelope entry naming a tool the declaration does not declare is refused
/// by validation, not merely by the code generator.
///
/// The generator checked this and validation did not, so a declaration could
/// pass `adi validate` and fail `adi generate-c`. That is the wrong way round:
/// validation is the cheap check run on every edit. The case that produces it is
/// a manufacturer copying an example, deleting a tool they do not have, and
/// leaving an interlock that still names it.
#[test]
fn an_envelope_naming_an_undeclared_tool_is_refused() {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/declarations/neutral-machine.yaml"),
    )
    .expect("the neutral fixture");

    let mut declaration: adi_protocol::Declaration =
        serde_yaml::from_str(&text).expect("it parses");

    assert!(declaration.validate().is_ok(), "unedited, it is valid");

    // Delete a tool the envelope refers to, as someone adapting a declaration
    // would. The neutral fixture's rate limit names this one.
    let limited = adi_protocol::ids::SemanticId::from("adi.example.process.start");
    declaration.tools.retain(|t| t.id != limited);

    let err = declaration
        .validate()
        .expect_err("an envelope naming a deleted tool must be refused");

    let text = err.to_string();
    assert!(
        text.contains("process.start"),
        "the message should name the missing tool: {text}"
    );
}
