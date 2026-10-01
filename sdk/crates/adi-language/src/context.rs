//! What the model is given, and how much of it.
//!
//! ADI-P 9.4.1 states what an interpreter must receive: the utterance, the
//! declared styles and materials, the decision history, and the cycles the
//! machine remembers. It does not state how much of each, and the difference
//! between those two is the difference between a response in two seconds and a
//! response in twenty.
//!
//! On an appliance-class processor the cost of a request is dominated by the
//! prompt, not by the reply. A reply is tens of tokens. A history assembled
//! without limit is thousands, and every one of them is processed before the
//! first token of the answer appears.
//!
//! So the context is assembled once, here, under a declared budget, rather than
//! by each backend to its own taste. A backend renders this into whatever
//! format its model expects. It does not decide what goes in.

use std::fmt::Write as _;

use adi_core::{DecisionRecord, Remembered};
use adi_decision::SelectionContext;
use adi_protocol::knowledge::Knowledge;

/// Limits on what is assembled.
///
/// These are not tuning parameters to be raised when a model seems confused.
/// Raising them raises latency on every request, including the ones that were
/// already correct.
#[derive(Debug, Clone, Copy)]
pub struct ContextBudget {
    /// Remembered cycles offered to the model, most recent first.
    ///
    /// Three covers "the last one", "the one before" and "the one this
    /// morning", which is the range of reference people actually use. A person
    /// who means a cycle further back than that names it.
    pub remembered_cycles: usize,

    /// Earlier decisions offered, most recent first. Enough for a model to see
    /// whether the last correction did anything.
    pub history: usize,

    /// Upper bound on the assembled prompt, in estimated tokens.
    ///
    /// Enforced by truncation of the lowest-value section first, so exceeding
    /// it degrades the context rather than failing the request.
    pub max_tokens: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            remembered_cycles: 3,
            history: 3,
            max_tokens: 1024,
        }
    }
}

/// Everything the model is given for one request, assembled and bounded.
#[derive(Debug, Clone, Default)]
pub struct ModelContext {
    /// What the person said.
    pub utterance: String,
    /// Style identifiers this machine's knowledge declares.
    pub styles: Vec<String>,
    /// Material identifiers this machine's knowledge declares.
    pub materials: Vec<String>,
    /// Variables the engine models, so a model knows what it may name.
    /// The variables the model may name, each with its unit and what it means.
    ///
    /// Names alone were sent until this. A model then had to infer from
    /// `shot_time` what a request about bitterness referred to, which works
    /// where the model was trained on the domain and not otherwise. A
    /// manufacturer's own variable has no such luck: nothing in a model's
    /// training says what `x.acme.tray_state` is.
    pub variables: Vec<VariableInfo>,
    /// Recent cycles, most recent first.
    pub remembered: Vec<RememberedSummary>,
    /// Recent decisions, most recent first.
    pub history: Vec<DecisionSummary>,
    /// Sections dropped to stay inside the budget, for the record.
    pub truncated: Vec<&'static str>,
}

/// One remembered cycle, reduced to what a model can use.
#[derive(Debug, Clone)]
pub struct RememberedSummary {
    pub cycle_id: String,
    pub style: Option<String>,
    /// Settings as `name=value unit`, which is what a relative target resolves
    /// against.
    pub settings: Vec<String>,
    pub feedback: Vec<String>,
}

/// One earlier decision, reduced to what a model can use.
#[derive(Debug, Clone)]
pub struct DecisionSummary {
    pub cycle_id: String,
    pub cause: String,
    pub tool: Option<String>,
}

/// Characters per token, for estimating prompt size without a tokeniser.
///
/// Every model tokenises differently, and a backend that has its own tokeniser
/// should use it. This is for the budget check, where an approximation that is
/// always available is more useful than an exact figure that is not. Four is
/// the usual figure for English prose; identifiers tokenise worse, so this
/// under-counts and the budget is therefore conservative in the safe
/// direction.
const CHARS_PER_TOKEN: usize = 4;

/// Estimated tokens in a string.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

/// A variable as the model is told about it.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableInfo {
    pub name: String,
    /// Absent where the profile models a variable its knowledge does not
    /// declare.
    pub unit: Option<String>,
    pub meaning: Option<String>,
}

impl ModelContext {
    /// Assemble what ADI-P 9.4.1 requires, within `budget`.
    pub fn assemble(
        utterance: &str,
        knowledge: &Knowledge,
        variables: &[adi_protocol::ids::VariableId],
        context: &SelectionContext<'_>,
        budget: ContextBudget,
    ) -> Self {
        let mut built = Self {
            utterance: utterance.to_string(),
            styles: knowledge.style_ids().into_iter().map(str::to_string).collect(),
            materials: knowledge.material_ids().into_iter().map(str::to_string).collect(),
            variables: variables
                .iter()
                .map(|v| {
                    let declared = knowledge.variable(v);
                    VariableInfo {
                        name: v.0.clone(),
                        unit: declared.map(|d| format!("{:?}", d.unit).to_lowercase()),
                        meaning: declared.map(|d| d.description.trim().to_string()),
                    }
                })
                .collect(),
            remembered: Vec::new(),
            history: Vec::new(),
            truncated: Vec::new(),
        };

        if let Some(memory) = context.memory {
            built.remembered = memory
                .recent(budget.remembered_cycles)
                .into_iter()
                .map(summarise_remembered)
                .collect();
        }

        built.history = context
            .history
            .iter()
            .take(budget.history)
            .map(summarise_decision)
            .collect();

        built.enforce(budget);
        built
    }

    /// Estimated size of the rendered context.
    pub fn estimated_tokens(&self) -> usize {
        estimate_tokens(&self.render())
    }

    /// Drop sections until the budget is met.
    ///
    /// The order is deliberate and runs from least to most load bearing. The
    /// utterance is never dropped: a request with no request is not a smaller
    /// request, it is a different one.
    fn enforce(&mut self, budget: ContextBudget) {
        if self.estimated_tokens() <= budget.max_tokens {
            return;
        }

        // History first. A model can interpret a request without knowing what
        // the engine decided last time; it cannot interpret "half of last
        // time" without knowing what last time was.
        if !self.history.is_empty() {
            self.history.clear();
            self.truncated.push("history");
            if self.estimated_tokens() <= budget.max_tokens {
                return;
            }
        }

        // Then older remembered cycles, newest kept.
        while self.remembered.len() > 1 && self.estimated_tokens() > budget.max_tokens {
            self.remembered.pop();
            if !self.truncated.contains(&"remembered") {
                self.truncated.push("remembered");
            }
        }

        // Then materials, which narrow a target rather than name a drink.
        if self.estimated_tokens() > budget.max_tokens && !self.materials.is_empty() {
            self.materials.clear();
            self.truncated.push("materials");
        }

        // Styles are last. Without them a model cannot name what was asked for
        // at all, and an intent naming an undeclared style is refused anyway.
        if self.estimated_tokens() > budget.max_tokens && !self.styles.is_empty() {
            self.styles.clear();
            self.truncated.push("styles");
        }
    }

    /// A plain rendering, used for the budget estimate and as the default a
    /// backend may send if it has nothing better.
    ///
    /// Deliberately not a prompt. It carries no instructions, no role framing
    /// and no output format, because those belong to the backend that knows
    /// which model it is talking to.
    pub fn render(&self) -> String {
        let mut out = String::new();

        let _ = writeln!(out, "request: {}", self.utterance);

        if !self.styles.is_empty() {
            let _ = writeln!(out, "styles: {}", self.styles.join(", "));
        }
        if !self.materials.is_empty() {
            let _ = writeln!(out, "materials: {}", self.materials.join(", "));
        }
        if !self.variables.is_empty() {
            let _ = writeln!(out, "variables:");
            for v in &self.variables {
                match (&v.unit, &v.meaning) {
                    (Some(unit), Some(meaning)) => {
                        let _ = writeln!(out, "  {} ({}): {}", v.name, unit, meaning);
                    }
                    // A variable the knowledge does not declare still has to be
                    // offered, since a profile may model one it has not
                    // described. Naming it alone is what used to happen to all
                    // of them.
                    _ => {
                        let _ = writeln!(out, "  {}", v.name);
                    }
                }
            }
        }

        for cycle in &self.remembered {
            let _ = write!(out, "cycle {}", cycle.cycle_id);
            if let Some(style) = &cycle.style {
                let _ = write!(out, " style={style}");
            }
            if !cycle.settings.is_empty() {
                let _ = write!(out, " {}", cycle.settings.join(" "));
            }
            if !cycle.feedback.is_empty() {
                let _ = write!(out, " said={}", cycle.feedback.join(","));
            }
            let _ = writeln!(out);
        }

        for decision in &self.history {
            let _ = writeln!(
                out,
                "decided {} {} {}",
                decision.cycle_id,
                decision.cause,
                decision.tool.as_deref().unwrap_or("none")
            );
        }

        out
    }
}

fn summarise_remembered(entry: &Remembered) -> RememberedSummary {
    RememberedSummary {
        cycle_id: entry.cycle_id.clone(),
        style: entry.style.clone(),
        settings: entry
            .settings
            .iter()
            .map(|(variable, quantity)| {
                format!("{}={}{:?}", variable.0, quantity.value, quantity.unit)
            })
            .collect(),
        feedback: entry.feedback.descriptors.clone(),
    }
}

fn summarise_decision(record: &DecisionRecord) -> DecisionSummary {
    DecisionSummary {
        cycle_id: record.cycle_id.clone(),
        cause: record.cause.clone(),
        tool: record.selected_tool.as_ref().map(|t| t.0.clone()),
    }
}
