//! The base domain profile.
//!
//! A profile normally carries what its domain knows. Espresso knows what a good
//! extraction is, which faults look alike, and which order to rule them out in.
//! This profile knows none of that, deliberately, and is for machines whose
//! domain nobody has written yet.
//!
//! Everything it reasons about arrives in the manufacturer's own knowledge
//! extension (ADI-P 6): the variables with their units and meanings, the named
//! styles with their targets, and the notes. A machine declaring
//! `domain_profile: "base/v1"` and supplying that one file can take spoken
//! requests, answer questions, and correct an outcome that drifts outside its
//! target, with no code written beyond its own firmware.
//!
//! What it cannot do is diagnose. Knowing that a result is wrong is arithmetic;
//! knowing why, and which correction would make it worse, is the domain's
//! knowledge and is what a profile is for. See `rules` for where that line
//! falls, and `HOW-TO-ADD-A-DOMAIN.md` for writing one.
//!
//! This is an ordinary profile and not a special case in the engine. It
//! implements the same trait espresso does and is selected the same way, by
//! the name the machine's declaration states, so the path a machine with no
//! domain takes is the path every machine takes.

pub mod rules;

use adi_core::cycle::Cycle;
use adi_core::domain::{Diagnosis, Domain, ExtensionRejected};
use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{
    Knowledge, KnowledgeBase, KnowledgeExtension, Layer, ResolvedTargets,
};

pub const PROFILE_ID: &str = "base/v1";

/// The base knowledge, compiled in. Empty, and a real file rather than a
/// literal so that both profiles are built the same way from the same format.
const BASE_KNOWLEDGE: &str = include_str!("../../../profiles/base/knowledge.yaml");

pub struct Base {
    variables: Vec<VariableId>,
    knowledge: Knowledge,
}

#[derive(Debug, thiserror::Error)]
pub enum BaseError {
    #[error("the base knowledge file does not parse: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error(transparent)]
    Extension(#[from] adi_protocol::ExtensionError),
    #[error(transparent)]
    Knowledge(#[from] adi_protocol::KnowledgeError),
}

impl Default for Base {
    fn default() -> Self {
        Self::new()
    }
}

impl Base {
    /// Panics only if the compiled-in base file is malformed, which is a build
    /// fault and is caught by this crate's tests.
    pub fn new() -> Self {
        Self::try_new().expect("the compiled-in base knowledge must be valid")
    }

    pub fn try_new() -> Result<Self, BaseError> {
        let base: KnowledgeBase = serde_yaml::from_str(BASE_KNOWLEDGE)?;
        let knowledge = Knowledge::new(base.into_layer())?;
        let variables = knowledge.variable_ids().into_iter().cloned().collect();
        Ok(Self {
            variables,
            knowledge,
        })
    }
}

impl Domain for Base {
    fn profile_id(&self) -> &str {
        PROFILE_ID
    }

    /// Empty until an extension supplies some. Every variable this profile
    /// knows was declared by the manufacturer, which is the whole point of it.
    fn variables(&self) -> &[VariableId] {
        &self.variables
    }

    fn knowledge(&self) -> &Knowledge {
        &self.knowledge
    }

    /// Nothing. Turning raw signal curves into outcomes is perception, and
    /// perception is specific to the machine: four espresso variables come out
    /// of one mass curve by rules that mean nothing anywhere else.
    ///
    /// An appliance that reports finished measurements needs none of this, and
    /// most can. One that reports only raw curves will measure nothing under
    /// this profile, and the diagnosis says so in as many words.
    fn derive(&self, _cycle: &mut Cycle) {}

    fn diagnose(&self, cycle: &Cycle, targets: &ResolvedTargets) -> Diagnosis {
        rules::diagnose(cycle, targets, &self.knowledge)
    }

    /// None. The words a person uses about a result are domain vocabulary:
    /// sour and bitter mean nothing to a dishwasher. A machine on this profile
    /// is corrected from what it measured and not from what it was called.
    fn feedback_vocabulary(&self) -> &[&str] {
        &[]
    }

    /// ADI-P 6.5: a rejected extension leaves the profile exactly as it was,
    /// so the variable list is rebuilt only after the layer has been accepted.
    fn add_extension(
        &mut self,
        extension: KnowledgeExtension,
        layer: Layer,
    ) -> Result<(), ExtensionRejected> {
        let base: KnowledgeBase = serde_yaml::from_str(BASE_KNOWLEDGE)
            .map_err(|_| ExtensionRejected::Unsupported(PROFILE_ID.to_string()))?;
        extension.validate(&base)?;
        self.knowledge.add(extension.into_layer(layer))?;
        self.variables = self.knowledge.variable_ids().into_iter().cloned().collect();
        Ok(())
    }
}
