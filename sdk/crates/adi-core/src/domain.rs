//! The `Domain` trait. Everything a profile must supply.
//!
//! Domain rules are code rather than data for now, deliberately. Designing a
//! rules language from one example produces a language that fits one example.
//! Extract a data format later, once a second domain exists to test it.
//!
//! Everything that is not rules stays as data: registry, targets, priors, step
//! sizes, constraints.

use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{Knowledge, ResolvedTargets};

use crate::cycle::Cycle;

/// A cause the profile can identify from a cycle.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnosis {
    /// Profile-defined cause identifier.
    pub cause: String,
    /// The variable the profile wants moved, if any. `None` means no
    /// actuation will help, for example a preparation fault the operator must
    /// fix themselves.
    pub target_variable: Option<VariableId>,
    /// Signed magnitude of the desired change, in the variable's own unit.
    pub desired_delta: Option<f64>,
    /// 0 to 1. Low confidence produces smaller corrections.
    pub confidence: f64,
    /// Human-readable reasoning, for the language layer to phrase. Never used
    /// as an actuation input.
    pub explanation: String,
}

impl Diagnosis {
    /// A diagnosis with nothing to correct. The cycle was good, or the fix is
    /// not something the machine can do.
    pub fn advisory(cause: impl Into<String>, explanation: impl Into<String>) -> Self {
        Self {
            cause: cause.into(),
            target_variable: None,
            desired_delta: None,
            confidence: 1.0,
            explanation: explanation.into(),
        }
    }
}

/// One domain profile.
pub trait Domain {
    /// Profile identifier, matching `domain_profile` in a declaration.
    fn profile_id(&self) -> &str;

    /// Variables this profile models. A skill response naming anything outside
    /// this list is ignored rather than guessed at.
    fn variables(&self) -> &[VariableId];

    /// What a registry tool is for, in words, where the profile says.
    ///
    /// A registry identifier means the same thing on every machine that declares
    /// it, so the description belongs with the profile rather than being retyped
    /// in each manufacturer's declaration. A description written on the tool
    /// takes precedence, since what is true of one mechanism is that
    /// manufacturer's to state, and a vendor tool has no entry here at all.
    ///
    /// Defaulted, so a profile that supplies none still compiles.
    fn tool_description(&self, _tool: &adi_protocol::ids::SemanticId) -> Option<&str> {
        None
    }

    /// The knowledge this machine reasons from: the domain base plus whatever
    /// vendor, deployment and user layers were accepted.
    ///
    /// Returned whole rather than as a `targets(style)` call, because resolution
    /// needs a style, a material and the layer stack together, and because a
    /// caller also needs step sizes, constraints and which variables are
    /// preferences. An earlier `base_targets(&self, style) -> ResolvedTargets`
    /// could not report an unknown style at all, so it fell back to a default
    /// and quietly made every drink the same one.
    fn knowledge(&self) -> &Knowledge;

    /// Derive modelled variables from raw traces. This is where a profile turns
    /// signals into meaning, and where most of its perception work lives.
    fn derive(&self, cycle: &mut Cycle);

    /// Identify what went wrong, if anything.
    ///
    /// Ordering matters and is the profile's responsibility. A profile that can
    /// distinguish a preparation fault from a settings fault must check the
    /// preparation fault first, because prescribing a settings change for a
    /// preparation fault makes the result worse.
    fn diagnose(&self, cycle: &Cycle, targets: &ResolvedTargets) -> Diagnosis;

    // Step sizes come from `knowledge().default_step()`, which carries the unit
    // and the layer that supplied it. A bare `f64` on this trait duplicated the
    // `default_steps` map in the knowledge base and could drift from it.

    /// Descriptors this profile accepts as subjective feedback, used to close
    /// the language layer's output schema.
    fn feedback_vocabulary(&self) -> &[&str];
}
