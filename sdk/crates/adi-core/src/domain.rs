//! The `Domain` trait. Everything a profile must supply.
//!
//! Domain rules are code rather than data for now, deliberately. Designing a
//! rules language from one example produces a language that fits one example.
//! Extract a data format later, once a second domain exists to test it.
//!
//! Everything that is not rules stays as data: registry, targets, priors, step
//! sizes, constraints.

use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{Knowledge, KnowledgeExtension, Layer, ResolvedTargets};

use crate::cycle::Cycle;

/// Why a knowledge extension was not accepted.
///
/// ADI-P 6 makes accepting an extension a property of a profile, so the
/// refusal belongs here with the trait and not inside one profile's error
/// type. Before this existed, loading an extension was an inherent method on
/// the espresso profile, which meant a caller holding a domain chosen at
/// runtime could not offer a manufacturer their own knowledge at all.
#[derive(Debug, thiserror::Error)]
pub enum ExtensionRejected {
    /// Malformed in itself: bad namespace, a redefinition, a missing parent.
    /// ADI-P 6.5 items 1 to 4 and 6.
    #[error(transparent)]
    Extension(#[from] adi_protocol::ExtensionError),
    /// Well formed and does not layer: it widens a constraint, or changes a
    /// unit. ADI-P 6.4.
    #[error(transparent)]
    Knowledge(#[from] adi_protocol::KnowledgeError),
    /// The profile has no base to layer onto. A profile that reasons from
    /// compiled-in knowledge always has one; a test double need not.
    #[error("the {0} profile accepts no knowledge extensions")]
    Unsupported(String),
}

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

    /// Which declared signal a modelled variable was derived from, where the
    /// profile can say.
    ///
    /// ADI-P 9.3 requirement 5 obliges the engine to retain the inputs to every
    /// decision, and a measured input is only meaningful alongside the signal it
    /// was read from. The engine cannot work that out for itself: `derive` turns
    /// traces into variables by rules the profile owns, and in espresso four
    /// variables come from one mass curve. Only the profile knows.
    ///
    /// Defaulted to `None`, which costs a profile that does not answer nothing
    /// worse than a record without measured provenance. Returning a signal the
    /// variable did not come from would be worse than returning nothing, since
    /// the record is read when something has already gone wrong.
    fn signal_for(&self, _variable: &VariableId) -> Option<adi_protocol::ids::SemanticId> {
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

    /// Accept a manufacturer's, integrator's or user's knowledge. ADI-P 6.
    ///
    /// Defaulted to a refusal so that a profile with no base knowledge, such
    /// as a test double, still compiles. Any profile a machine actually runs
    /// on should implement it, because ADI-P 6 gives a manufacturer the right
    /// to extend whatever profile they declare.
    ///
    /// ADI-P 6.5 governs what a caller does with the error: report it and
    /// carry on with the knowledge already held. A rejected extension
    /// degrades a machine to standard behaviour and never stops it working,
    /// so an implementation must leave the profile exactly as it was.
    fn add_extension(
        &mut self,
        _extension: KnowledgeExtension,
        _layer: Layer,
    ) -> Result<(), ExtensionRejected> {
        Err(ExtensionRejected::Unsupported(self.profile_id().to_string()))
    }
}
