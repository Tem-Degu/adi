//! Layer resolution. PROTOCOL.md section 6.
//!
//! Fictional domain, for the reason given in `knowledge.rs`. The espresso
//! knowledge exercises the same code in `adi-domain-espresso`.

use adi_protocol::knowledge::KnowledgeError;
use adi_protocol::{
    Knowledge, KnowledgeBase, KnowledgeExtension, Layer, Range, StyleEntry, Unit,
};
use std::collections::BTreeMap;

fn load<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/knowledge")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}

fn base() -> KnowledgeBase {
    load("neutral-base.yaml")
}

fn extension() -> KnowledgeExtension {
    load("neutral-extension.yaml")
}

fn base_only() -> Knowledge {
    Knowledge::new(base().into_layer()).expect("a base layer")
}

fn with_vendor() -> Knowledge {
    let mut k = base_only();
    k.add(extension().into_layer(Layer::Vendor))
        .expect("the shipped extension should be accepted");
    k
}

fn range(min: f64, max: f64, unit: Unit) -> Range {
    Range {
        min,
        max,
        resolution: None,
        unit,
    }
}

// ------------------------------------------------------------ construction

#[test]
fn knowledge_must_start_from_a_base() {
    let not_base = extension().into_layer(Layer::Vendor);
    assert!(matches!(
        Knowledge::new(not_base),
        Err(KnowledgeError::NotBase(Layer::Vendor))
    ));
}

#[test]
fn a_layer_cannot_be_supplied_twice() {
    let mut k = with_vendor();
    assert!(matches!(
        k.add(extension().into_layer(Layer::Vendor)),
        Err(KnowledgeError::DuplicateLayer(Layer::Vendor))
    ));
}

/// Layers narrow in order. Accepting a lower layer after a higher one would let
/// a vendor constraint be applied on top of a user preference and silently
/// reverse the precedence in 6.1.
#[test]
fn layers_cannot_arrive_out_of_order() {
    let mut k = base_only();
    k.add(extension().into_layer(Layer::User)).unwrap();
    assert!(matches!(
        k.add(extension().into_layer(Layer::Vendor)),
        Err(KnowledgeError::OutOfOrder { .. })
    ));
}

// ------------------------------------------------------------- inheritance

/// The distinction the whole design turns on.
///
/// In the fixture, `short` inherits from `standard` and sets output_ratio 1.2
/// to 1.8 against the parent's 1.8 to 2.4. Those ranges touch at a single point.
/// If inheritance intersected instead of overriding, the child would resolve to
/// the degenerate range 1.8 to 1.8.
#[test]
fn a_child_style_overrides_its_parent_rather_than_narrowing_it() {
    let r = base_only().resolve("adi.example.style.short", None).unwrap();
    let ratio = r.targets.get("output_ratio").expect("output_ratio");
    assert_eq!(
        (ratio.value.min, ratio.value.max),
        (1.2, 1.8),
        "the child's own range must survive intact"
    );
}

/// A field the child does not name is inherited unchanged.
#[test]
fn an_unnamed_field_is_inherited() {
    let k = base_only();
    let parent = k.resolve("adi.example.style.standard", None).unwrap();
    let child = k.resolve("adi.example.style.short", None).unwrap();

    // The child names cycle_time and output_ratio but not process_temperature,
    // so only the constraint applies there, identically for both.
    assert_eq!(
        parent.targets.get("process_temperature").map(|a| a.value.max),
        child.targets.get("process_temperature").map(|a| a.value.max)
    );
    assert_eq!(child.targets.get("cycle_time").unwrap().value.min, 22.0);
}

#[test]
fn an_unknown_style_is_reported_not_guessed() {
    assert!(matches!(
        base_only().resolve("adi.example.style.nothing", None),
        Err(KnowledgeError::UnknownStyle(_))
    ));
}

/// A cycle introduced after validation must still be reported rather than
/// overflowing the stack, since resolution runs on every cycle and validation
/// only once.
#[test]
fn a_cycle_at_resolution_time_is_reported() {
    let mut b = base();
    b.styles = vec![
        StyleEntry {
            id: "a".into(),
            description: None,
            inherits: Some("b".into()),
            targets: BTreeMap::new(),
        },
        StyleEntry {
            id: "b".into(),
            description: None,
            inherits: Some("a".into()),
            targets: BTreeMap::new(),
        },
    ];
    let k = Knowledge::new(b.into_layer()).unwrap();
    assert!(matches!(
        k.resolve("a", None),
        Err(KnowledgeError::InheritanceCycle(_))
    ));
}

// ----------------------------------------------------------------- layering

#[test]
fn a_vendor_may_narrow_a_base_constraint() {
    let base_max = base_only()
        .constraint("process_temperature")
        .unwrap()
        .value
        .max;
    let vendor = with_vendor();
    let c = vendor.constraint("process_temperature").unwrap();

    assert_eq!(base_max, 98.0);
    assert_eq!(c.value.max, 96.0, "the vendor's narrower ceiling must win");
    assert_eq!(c.layer, Layer::Vendor, "and must be attributed to it");
}

#[test]
fn a_vendor_may_not_widen_a_base_constraint() {
    let mut e = extension();
    e.constraints
        .insert("process_temperature".into(), range(50.0, 120.0, Unit::C));

    let mut k = base_only();
    let before = k.constraint("process_temperature").unwrap().value.max;
    let err = k.add(e.into_layer(Layer::Vendor)).unwrap_err();

    assert!(matches!(
        err,
        KnowledgeError::Monotonicity(adi_protocol::knowledge::MonotonicityError::Widened { .. })
    ));
    assert_eq!(
        k.constraint("process_temperature").unwrap().value.max,
        before,
        "a rejected layer must leave nothing behind"
    );
}

/// Section 6.5: a failing extension is rejected with a reason and the engine
/// continues on the knowledge it already has. A bad vendor file must not brick
/// an appliance.
#[test]
fn a_rejected_layer_leaves_the_machine_working() {
    let mut e = extension();
    e.constraints
        .insert("cycle_time".into(), range(1.0, 600.0, Unit::S));

    let mut k = base_only();
    assert!(k.add(e.into_layer(Layer::Vendor)).is_err());

    k.resolve("adi.example.style.standard", None)
        .expect("base knowledge must still resolve after a rejected layer");
    assert_eq!(k.layers().len(), 1);
}

/// Intersecting ranges of different units would produce a number that means
/// nothing. This is the same hole that was found and fixed in `adi-safety`
/// accumulation, in a different place.
#[test]
fn a_layer_cannot_change_the_unit_of_a_constraint() {
    let mut e = extension();
    e.constraints
        .insert("cycle_time".into(), range(20.0, 30.0, Unit::Ms));

    let mut k = base_only();
    assert!(matches!(
        k.add(e.into_layer(Layer::Vendor)),
        Err(KnowledgeError::Monotonicity(
            adi_protocol::knowledge::MonotonicityError::UnitMismatch { .. }
        ))
    ));
}

// --------------------------------------------------------------- attribution

/// 9.3 asks which layer produced the effective value, not which one mentioned
/// it last. A layer that restates what already held has narrowed nothing.
#[test]
fn restating_a_range_does_not_claim_credit_for_it() {
    let mut e = extension();
    e.constraints
        .insert("cycle_time".into(), range(15.0, 60.0, Unit::S));

    let mut k = base_only();
    k.add(e.into_layer(Layer::Vendor)).unwrap();

    assert_eq!(
        k.constraint("cycle_time").unwrap().layer,
        Layer::Base,
        "the base still owns this bound; the vendor only repeated it"
    );
}

#[test]
fn a_style_target_is_attributed_to_the_layer_that_declared_it() {
    let k = with_vendor();
    let r = k.resolve("x.acme.style.custom", None).unwrap();

    assert_eq!(r.targets.get("cycle_time").unwrap().layer, Layer::Vendor);
    // Inherited from the base style, so the base keeps the credit.
    assert_eq!(r.targets.get("output_ratio").unwrap().layer, Layer::Base);
}

// ------------------------------------------------------------------ material

#[test]
fn a_material_prior_narrows_targets_and_supplies_offsets() {
    let k = base_only();
    let plain = k.resolve("adi.example.style.standard", None).unwrap();
    let with_material = k
        .resolve("adi.example.style.standard", Some("adi.example.material.light"))
        .unwrap();

    assert_eq!(
        plain.targets.get("process_temperature").unwrap().value.min,
        85.0,
        "without a material, only the constraint applies"
    );
    assert_eq!(
        with_material
            .targets
            .get("process_temperature")
            .unwrap()
            .value
            .min,
        92.0,
        "the prior narrows it"
    );
    assert_eq!(
        with_material.offsets.get("position").map(|a| a.value),
        Some(-3.0)
    );
    assert!(plain.offsets.is_empty());
}

/// A prior is a second opinion about the batch, not a bound on the style, so it
/// may lean past the style's range. Resolution takes the overlap.
///
/// This is the one place composition differs from layering, where reaching
/// outside is a rejected layer. See `ResolvedTargets::narrow_to`.
#[test]
fn a_prior_may_lean_past_the_style_and_resolve_to_the_overlap() {
    let mut b = base();
    // The style allows 25 to 32; the prior wants 30 to 40.
    b.material_priors[0]
        .targets
        .insert("cycle_time".into(), range(30.0, 40.0, Unit::S));
    let k = Knowledge::new(b.into_layer()).unwrap();

    let r = k
        .resolve(
            "adi.example.style.standard",
            Some("adi.example.material.light"),
        )
        .unwrap();
    let t = &r.targets.get("cycle_time").unwrap().value;
    assert_eq!((t.min, t.max), (30.0, 32.0));
}

/// Overlap is required. With no value satisfying both, picking one silently
/// would make the machine claim a target nobody set.
#[test]
fn a_prior_disjoint_from_the_style_is_reported() {
    let mut b = base();
    b.material_priors[0]
        .targets
        .insert("cycle_time".into(), range(40.0, 50.0, Unit::S));
    let k = Knowledge::new(b.into_layer()).unwrap();

    assert!(matches!(
        k.resolve(
            "adi.example.style.standard",
            Some("adi.example.material.light")
        ),
        Err(KnowledgeError::Monotonicity(_))
    ));
}

/// A style, unlike a prior, may not reach outside the hard envelope. The
/// envelope is what the machine can actually hold.
#[test]
fn a_style_reaching_outside_its_constraint_is_reported() {
    let mut b = base();
    // The constraint allows 15 to 60; this style asks for 70.
    b.styles[0]
        .targets
        .insert("cycle_time".into(), range(25.0, 70.0, Unit::S));
    let k = Knowledge::new(b.into_layer()).unwrap();

    assert!(matches!(
        k.resolve("adi.example.style.standard", None),
        Err(KnowledgeError::Monotonicity(_))
    ));
}

#[test]
fn an_unknown_material_is_reported() {
    assert!(matches!(
        base_only().resolve("adi.example.style.standard", Some("nope")),
        Err(KnowledgeError::UnknownMaterial(_))
    ));
}

// ------------------------------------------------------- steps, preferences

#[test]
fn the_highest_layer_wins_a_default_step() {
    let mut e = extension();
    e.defaults[0].variable = adi_protocol::VariableId("cycle_time".into());
    e.defaults[0].value = 5.0;
    e.defaults[0].unit = Unit::S;

    let mut k = base_only();
    assert_eq!(k.default_step("cycle_time").unwrap().value.value, 2.0);
    k.add(e.into_layer(Layer::Vendor)).unwrap();

    let step = k.default_step("cycle_time").unwrap();
    assert_eq!(step.value.value, 5.0);
    assert_eq!(step.layer, Layer::Vendor);
}

#[test]
fn only_the_base_decides_what_is_a_preference() {
    let k = with_vendor();
    assert!(k.is_preference("additive_volume"));
    assert!(
        !k.is_preference("cycle_time"),
        "a process variable must stay within the engine's reach"
    );
}

#[test]
fn styles_from_every_layer_are_offered() {
    let k = with_vendor();
    let ids = k.style_ids();
    assert!(ids.contains(&"adi.example.style.standard"));
    assert!(ids.contains(&"x.acme.style.custom"));
    assert_eq!(ids.len(), 3);
}
