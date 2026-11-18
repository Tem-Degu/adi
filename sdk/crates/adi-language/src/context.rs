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
use adi_decision::{SelectionContext, Turn};
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

    /// Exchanges offered, OLDEST first, so they read as a conversation.
    ///
    /// The other way round from `history`, which is most recent first because
    /// nothing reads it as prose.
    pub conversation: usize,

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
            // Four exchanges, which is two or three sentences each way - about
            // sixty tokens against a default budget of 1024. Enough for "that
            // one" to have something to point at.
            conversation: 4,
            max_tokens: 1024,
        }
    }
}

/// Everything the model is given for one request, assembled and bounded.
#[derive(Debug, Clone, Default)]
pub struct ModelContext {
    /// What the person said.
    pub utterance: String,

    /// The exchanges before this one, oldest first.
    ///
    /// ADI-P 9.4.1 requires THE utterance, singular, and the decision history.
    /// The records are the governed memory and say what was done; they are not
    /// a conversation, and a sentence that points back at one ("another one",
    /// "the second one") cannot be resolved from a tool name and a volume.
    ///
    /// Carries no authority over values. A quantity still comes from a style,
    /// a skill response or the declared envelope, and the schema a model
    /// answers in cannot express one that is not clamped and checked.
    pub recent: Vec<Turn>,
    /// Styles this machine's knowledge declares, each with what it is.
    ///
    /// Carried with a description for the same reason a variable is carried
    /// with its meaning: a request names a style the way a person would, and
    /// an identifier alone can be mapped onto one only by a model that already
    /// knows the domain.
    pub styles: Vec<StyleInfo>,
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
    /// What the appliance is reading now, one entry per outcome it measures.
    ///
    /// Where the machine stands. A request may be stated entirely against it:
    /// "a bit warmer" names no number, and without a reading there is nothing
    /// for warmer to be warmer than. A model given only past cycles has to
    /// refuse such a request on a machine that has run none.
    pub measured: Vec<ReadingLine>,
    /// Recent cycles, most recent first.
    pub remembered: Vec<RememberedSummary>,
    /// Recent decisions, most recent first.
    pub history: Vec<DecisionSummary>,
    /// Sections dropped to stay inside the budget, for the record.
    /// What the machine can say about itself and its domain, beyond the
    /// structured data. Supplied by the manufacturer; see `knowledge::Note`.
    pub notes: Vec<NoteLine>,
    /// What this machine is. Already known to the engine and, until this was
    /// sent, unreachable to the person asking.
    pub device: Option<String>,
    /// Actions an operator may name, in the order the engine numbered them.
    /// ADI-P 8.2.
    ///
    /// A position in this list is what a model returns in place of a tool
    /// name, so the list and the schema's permitted indices are the same list.
    /// Derived from the declaration rather than from knowledge, which is why
    /// `assemble` leaves it empty and the caller fills it, as with `device`.
    pub actions: Vec<ActionLine>,
    pub truncated: Vec<&'static str>,
}

/// One action as the model is told about it. Its index is its position.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActionLine {
    /// The word a person would say. The tool's declared name.
    pub name: String,
    pub description: Option<String>,
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

    /// Which recipe that tool was serving, where it was serving one.
    ///
    /// THE FIELD THAT MAKES "ANOTHER ONE" ANSWERABLE. The tool and the
    /// arguments say a brew ran and that 118 millilitres went through it. On a
    /// machine whose one brewer serves a ristretto, an espresso, a doppio, a
    /// coffee, a lungo and an americano, that does not say which was made.
    ///
    /// So a model asked for "another one" after a lungo had a brew and a
    /// volume to go on, and answered with one of the machine's NAMED products
    /// - the only past decisions whose records identify themselves. It was not
    /// forgetting. The drink had never been written down.
    pub style: Option<String>,

    /// What was actually sent, with units.
    ///
    /// THE FIELD THAT MAKES A FOLLOW-UP ANSWERABLE. This summary carried the
    /// cycle, the cause and the tool and nothing else, so a model could see
    /// that a brew happened and not that it was 116 ml. Asked "why 38
    /// millilitres?" it had no idea 38 had ever been sent.
    ///
    /// The session keeps whole records - every argument, every input, the
    /// layer behind each value - and this threw almost all of it away on the
    /// way to the model. A machine that can be asked why it did that, and
    /// answers from its own record, is the thing being claimed; it cannot do
    /// that from three strings.
    pub arguments: Vec<ArgumentSummary>,

    /// Where the envelope moved a value, what was asked for before it did.
    /// Worth carrying: "I could not go that high" is a different answer from
    /// "that is what I chose".
    pub asked_for: Vec<ArgumentSummary>,

    /// Every figure that fed the decision, and which layer it came from.
    /// This is the actual answer to "why": not the number, but where the
    /// number came from.
    pub inputs: Vec<InputSummary>,

    /// Who chose the tool: the engine, the model, or a person naming it.
    pub chose: &'static str,

    /// What the domain rules wrote when they diagnosed. Flat, and true.
    pub note: String,
}

/// One argument, or one figure that was asked for and clamped.
#[derive(Debug, Clone)]
pub struct ArgumentSummary {
    pub parameter: String,
    pub value: f64,
    pub unit: String,
}

/// One value that fed a decision, with its provenance.
#[derive(Debug, Clone)]
pub struct InputSummary {
    pub variable: String,
    pub value: f64,
    pub unit: String,
    /// base knowledge, the manufacturer, this deployment, or something learned.
    pub layer: String,
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

/// A note as the model is told about it.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteLine {
    pub id: String,
    pub text: String,
}

/// A style as the model is told about it.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleInfo {
    pub name: String,
    /// Absent where the knowledge declares a style and says nothing about it.
    pub description: Option<String>,

    /// What the recipe actually asks for, as "90 to 130 g".
    ///
    /// A style used to reach the model as a NAME and a sentence of prose, and
    /// never its figures. So a model asked to supply a quantity alongside a
    /// style was supplying it blind: offered a lungo, whose recipe is 90 to
    /// 130 grams, one proposed 85. It was not ignoring the band. It had never
    /// been shown it.
    ///
    /// The engine clips such a figure into the recipe either way, because a
    /// prompt is advice and the envelope has to hold regardless. This is the
    /// other half: it stops the guess being necessary, and it lets the model
    /// see that a lungo has a figure for what ends up in the cup and none for
    /// anything else - which is what sent it reaching for a hot water step
    /// that belongs to a different drink.
    pub targets: Vec<(String, String)>,
}

/// A variable as the model is told about it.
/// One outcome as the appliance currently reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadingLine {
    pub name: String,
    pub value: f64,
    pub unit: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VariableInfo {
    pub name: String,
    /// Absent where the profile models a variable its knowledge does not
    /// declare.
    pub unit: Option<String>,
    pub meaning: Option<String>,
}

/// The last `n` exchanges, keeping their order.
///
/// `history` is truncated with `take`, because it arrives most recent first
/// and the front is what matters. A conversation arrives OLDEST first, so the
/// end is what matters and taking from the front would hand a model the start
/// of a conversation and hide what was just said.
fn last_of(turns: &[Turn], n: usize) -> Vec<Turn> {
    if n == 0 {
        return Vec::new();
    }
    let from = turns.len().saturating_sub(n);
    turns[from..].to_vec()
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
            recent: last_of(context.recent, budget.conversation),
            styles: knowledge
                .styles_described()
                .into_iter()
                .map(|(name, description)| StyleInfo {
                    name: name.to_string(),
                    description: description.map(str::to_string),
                    // The bands are filled in by whoever has a resolver to
                    // hand. This path builds a context from the knowledge
                    // alone and resolving every style here would make
                    // assembling one cost a resolution per drink, so the tool
                    // that already resolves them supplies them instead.
                    targets: Vec::new(),
                })
                .collect(),
            materials: knowledge.material_ids().into_iter().map(str::to_string).collect(),
            variables: variables
                .iter()
                .map(|v| {
                    let declared = knowledge.variable(v);
                    VariableInfo {
                        name: v.0.clone(),
                        unit: declared.map(|d| d.unit.as_str().to_string()),
                        meaning: declared.map(|d| d.description.trim().to_string()),
                    }
                })
                .collect(),
            measured: readings(context),
            remembered: Vec::new(),
            history: Vec::new(),
            // Every note, not a selection. A filter guesses what the question
            // needs, and a wrong guess withholds the answer without saying so.
            notes: knowledge
                .notes()
                .map(|n| NoteLine {
                    id: n.id.clone(),
                    text: n.text.trim().to_string(),
                })
                .collect(),
            device: None,
            // Derived from the declaration, which this does not have. The
            // caller fills both, as it does `device`.
            actions: Vec::new(),
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
    /// Bring the context within a budget, reporting what was dropped.
    ///
    /// Public because a caller that assembles a context itself, as the process
    /// backend does to avoid keeping the knowledge alive for a session, must
    /// still obey the budget. It did not, and notes could overflow the model's
    /// context with nothing reporting it.
    pub fn enforce(&mut self, budget: ContextBudget) {
        if self.estimated_tokens() <= budget.max_tokens {
            return;
        }

        // Notes first. They are prose and the largest thing here, and losing
        // them degrades an answer to "I don't have that information" while
        // losing anything else degrades a request that moves a machine.
        //
        // Reaching this at all means the notes do not fit the model this engine
        // is configured for, which is a packaging fault rather than a runtime
        // condition: whoever wrote them is responsible for the total size. The
        // engine degrades instead of failing, and says so in `truncated` so the
        // fault is visible rather than silent.
        //
        // Dropped oldest-layer-first rather than wholesale, so a vendor's note
        // about their own machine outlives a general one from the base.
        while !self.notes.is_empty() && self.estimated_tokens() > budget.max_tokens {
            self.notes.remove(0);
            if !self.truncated.contains(&"notes") {
                self.truncated.push("notes");
            }
        }
        if self.estimated_tokens() <= budget.max_tokens {
            return;
        }

        // History next. A model can interpret a request without knowing what
        // the engine decided last time; it cannot interpret "half of last
        // time" without knowing what last time was.
        // Then the conversation, before the decision history. Both are
        // memory, and of the two this is the one the engine can afford to
        // lose: a record is attributed and is what an answer about a figure is
        // built from, while an exchange only helps work out which thing was
        // meant. A model that has lost the conversation asks which drink; one
        // that has lost the history cannot say why it did anything.
        while !self.recent.is_empty() && self.estimated_tokens() > budget.max_tokens {
            self.recent.remove(0);
            if !self.truncated.contains(&"conversation") {
                self.truncated.push("conversation");
            }
        }
        if self.estimated_tokens() <= budget.max_tokens {
            return;
        }

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

        // Actions are not dropped at any point, and are deliberately absent
        // from the order above.
        //
        // Every other section degrades safely when it goes: a request naming a
        // style that was dropped is refused, because the style is not declared
        // as far as the model could see. An action does not degrade that way.
        // The permitted indices are baked into the schema from the count, so
        // dropping the list leaves a model able to name index 1 with nothing
        // to tell it what index 1 is - and index 1 is a physical act. Choosing
        // blindly between stopping a drink and rinsing the group is worse than
        // any amount of truncation.
        //
        // They are also the smallest section here: a name and a line each, for
        // the handful of tools that take no arguments.
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

        if let Some(device) = &self.device {
            let _ = writeln!(out, "machine: {device}");
        }

        if !self.notes.is_empty() {
            let _ = writeln!(out, "notes:");
            for note in &self.notes {
                let _ = writeln!(out, "  [{}] {}", note.id, note.text);
            }
        }

        if !self.styles.is_empty() {
            let _ = writeln!(out, "styles:");
            for style in &self.styles {
                match &style.description {
                    Some(description) => {
                        let _ = writeln!(out, "  {}: {}", style.name, description);
                    }
                    None => {
                        let _ = writeln!(out, "  {}", style.name);
                    }
                }
            }
        }
        // Numbered, because the number is what a model returns. Rendered next
        // to the styles so the two namable vocabularies sit together: a style
        // is a drink to ask for, an action is a thing to ask for.
        if !self.actions.is_empty() {
            let _ = writeln!(out, "actions, by number:");
            for (index, action) in self.actions.iter().enumerate() {
                match &action.description {
                    Some(description) => {
                        let _ = writeln!(out, "  {index}: {} - {description}", action.name);
                    }
                    None => {
                        let _ = writeln!(out, "  {index}: {}", action.name);
                    }
                }
            }
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

        if !self.measured.is_empty() {
            let _ = writeln!(out, "measured now:");
            for reading in &self.measured {
                let _ = writeln!(out, "  {} = {} {}", reading.name, reading.value, reading.unit);
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

/// The appliance's current readings, as the model is told them.
///
/// Empty where the appliance reports none, which is conforming and leaves a
/// request stated against the present state unanswerable. That is the honest
/// outcome: the alternative is a model inventing a number to be relative to.
fn readings(context: &SelectionContext<'_>) -> Vec<ReadingLine> {
    let Some(measured) = context.measured else {
        return Vec::new();
    };

    measured
        .iter()
        .map(|(variable, quantity)| ReadingLine {
            name: variable.0.clone(),
            value: quantity.value,
            unit: quantity.unit.as_str().to_string(),
        })
        .collect()
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

/// Summarise one decision for a model.
///
/// Public because `ProcessInterpreter` was building this struct by hand, which
/// is how it came to carry three fields while the record carried ten. Two
/// copies of the same mapping is two places to forget to widen, and one of
/// them was forgotten.
pub fn summarise_decision(record: &DecisionRecord) -> DecisionSummary {
    use adi_core::record::SelectedBy;

    DecisionSummary {
        cycle_id: record.cycle_id.clone(),
        cause: record.cause.clone(),
        tool: record.selected_tool.as_ref().map(|t| t.0.clone()),
        // What that tool was making. Without it a model looking back sees a
        // brew and a number of millilitres, and "another one" has nothing to
        // name.
        style: record.style.clone(),

        arguments: record
            .arguments
            .iter()
            .map(|(parameter, quantity)| ArgumentSummary {
                parameter: parameter.clone(),
                value: quantity.value,
                unit: format!("{:?}", quantity.unit).to_lowercase(),
            })
            .collect(),

        asked_for: record
            .clamped_from
            .iter()
            .map(|(parameter, wanted)| ArgumentSummary {
                parameter: parameter.clone(),
                value: *wanted,
                // The unit of the argument it was clamped to, where that
                // argument is still here to ask.
                unit: record
                    .arguments
                    .get(parameter)
                    .map(|q| format!("{:?}", q.unit).to_lowercase())
                    .unwrap_or_default(),
            })
            .collect(),

        inputs: record
            .inputs
            .iter()
            .map(|input| InputSummary {
                variable: input.variable.0.clone(),
                value: input.value.value,
                unit: format!("{:?}", input.value.unit).to_lowercase(),
                layer: layer_name(input.layer).to_string(),
            })
            .collect(),

        chose: match record.selected_by {
            SelectedBy::Engine => "the engine, from the declaration",
            SelectedBy::Model => "the model, from what the engine offered",
            SelectedBy::Operator => "a person, by naming it",
        },

        note: record.explanation.clone(),
    }
}

/// A layer in words rather than as a variant name.
///
/// `User` is "learned from you" and saying so out loud matters: a person asking
/// why is owed the answer "because you had it that way", not a number with no
/// history.
fn layer_name(layer: adi_protocol::knowledge::Layer) -> &'static str {
    use adi_protocol::knowledge::Layer;
    match layer {
        Layer::Base => "base knowledge",
        Layer::Vendor => "the manufacturer",
        Layer::Deployment => "this deployment",
        Layer::User => "something learned from you",
    }
}
