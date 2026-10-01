//! The espresso domain profile.
//!
//! This is the only crate in the SDK permitted to know what coffee is.

pub mod registry;
pub mod rules;

use adi_core::cycle::Cycle;
use adi_core::domain::{Diagnosis, Domain};
use adi_protocol::ids::VariableId;
use adi_protocol::knowledge::{Knowledge, KnowledgeBase, KnowledgeExtension, Layer, ResolvedTargets};

/// The base knowledge, compiled in.
///
/// Embedded rather than read from disk because the engine runs on an appliance
/// module with no filesystem to read from, and because the knowledge a build
/// shipped with should be a property of that build rather than of whatever
/// happens to be next to the binary.
const BASE_KNOWLEDGE: &str = include_str!("../../../profiles/espresso/knowledge.yaml");

pub struct Espresso {
    variables: Vec<VariableId>,
    knowledge: Knowledge,
}

#[derive(Debug, thiserror::Error)]
pub enum EspressoError {
    #[error("the espresso knowledge base does not parse: {0}")]
    Parse(#[from] serde_yaml::Error),
    /// The extension is malformed in itself: bad namespace, redefinition,
    /// a missing parent. ADI-P 6.5 items 1 to 4 and 6.
    #[error(transparent)]
    Extension(#[from] adi_protocol::ExtensionError),
    /// The extension is well formed but does not layer: it widens a constraint,
    /// or changes a unit. ADI-P 6.4.
    #[error(transparent)]
    Knowledge(#[from] adi_protocol::KnowledgeError),
}

impl Default for Espresso {
    fn default() -> Self {
        Self::new()
    }
}

impl Espresso {
    /// Panics only if the compiled-in knowledge base is malformed, which is a
    /// build fault rather than a runtime one and is caught by the tests in this
    /// crate. `try_new` is available where a caller would rather handle it.
    pub fn new() -> Self {
        Self::try_new().expect("the compiled-in espresso knowledge base must be valid")
    }

    pub fn try_new() -> Result<Self, EspressoError> {
        let base: KnowledgeBase = serde_yaml::from_str(BASE_KNOWLEDGE)?;

        // Read from the knowledge base rather than held here. The two used to be
        // separate, a list in this file and a table in PROTOCOL.md 7.3, with
        // nothing checking they agreed.
        let variables: Vec<VariableId> = base.variables.keys().cloned().collect();

        Ok(Self {
            variables,
            knowledge: Knowledge::new(base.into_layer())?,
        })
    }

    /// Accept a manufacturer's or integrator's knowledge extension.
    ///
    /// Validated structurally first, then for constraint monotonicity as it is
    /// layered. A rejected extension leaves the profile exactly as it was, per
    /// ADI-P 6.5: the machine keeps working on the knowledge it already has.
    pub fn add_extension(
        &mut self,
        extension: KnowledgeExtension,
        layer: Layer,
    ) -> Result<(), EspressoError> {
        let base: KnowledgeBase = serde_yaml::from_str(BASE_KNOWLEDGE)?;
        extension.validate(&base)?;
        self.knowledge.add(extension.into_layer(layer))?;
        Ok(())
    }

}

impl Domain for Espresso {
    fn profile_id(&self) -> &str {
        registry::PROFILE_ID
    }

    fn variables(&self) -> &[VariableId] {
        &self.variables
    }

    fn knowledge(&self) -> &Knowledge {
        &self.knowledge
    }

    fn derive(&self, cycle: &mut Cycle) {
        rules::derive(cycle);
    }

    fn diagnose(&self, cycle: &Cycle, targets: &ResolvedTargets) -> Diagnosis {
        rules::diagnose(cycle, targets)
    }

    fn feedback_vocabulary(&self) -> &[&str] {
        registry::FEEDBACK_VOCABULARY
    }
}
