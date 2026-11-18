//! `adi run`: a live session against a real appliance.
//!
//! Everything else in this tool inspects files or replays a corpus. This
//! starts an appliance, opens a session, and lets a person drive it, which is
//! the only way to see what the SDK actually does.
//!
//! The appliance is a real build of `libadip` running as a separate process.
//! Every byte crosses the same framing, CRC, decoding, range validation,
//! interlock check and stateful envelope that would run on a machine. Nothing
//! here is simulated except the mechanism at the far end.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adi_core::{Cycle, SubjectiveFeedback};
use adi_decision::{
    AutonomyLevel, Interpreter, ModelActionSelector, ModelToolSelector, SelectionContext,
    Understood,
};
use adi_language::context::VariableInfo;
use adi_language::context::ContextBudget;
use adi_language::{Explainer, Language, NoSpeaker, RecordExplainer, Speaker, Transcriber};
use adi_models::{
    ProcessExplainer, ProcessLearner, ProcessSpeaker, ProcessTranscriber, SeamConfig,
    Surroundings,
    Backend, ModelsConfig, ProcessBackend, ProcessChooser, ProcessInterpreter, ScriptedInterpreter,
    Seam,
};
use adi_protocol::declaration::Declaration;
use adi_protocol::ids::VariableId;
use adi_protocol::skill::Skill;
use adi_protocol::units::{Quantity, Unit};
use adi_session::{ChildAppliance, Session};

use crate::serial::SerialAppliance;

/// Everything `adi run` was asked for.
///
/// Gathered into one value because the list grew past the point where a
/// positional argument list is readable, and a caller passing two `Option`
/// paths in the wrong order would compile.
pub struct Options {
    pub declaration_path: PathBuf,
    pub skill_path: PathBuf,
    /// An appliance to start as a child process.
    pub appliance_path: Option<PathBuf>,
    /// A serial port a real appliance is listening on. Mutually exclusive
    /// with `appliance_path`.
    pub serial_path: Option<String>,
    pub baud: u32,
    /// A manufacturer's own knowledge, layered over the domain profile.
    pub knowledge_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub level: Option<String>,
}

pub fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let Options {
        declaration_path,
        skill_path,
        appliance_path,
        serial_path,
        baud,
        knowledge_path,
        config_path,
        level,
    } = options;

    let declaration: Declaration = serde_yaml::from_str(&read(&declaration_path)?)?;
    let skill: Skill = serde_yaml::from_str(&read(&skill_path)?)?;
    // The profile the machine asks for. ADI-P 4.1 makes `domain_profile` a
    // required field, so this is always stated and never guessed.
    let mut domain = crate::domain::for_profile(&declaration.domain_profile)?;
    println!("  profile      {}", domain.profile_id());

    // A skill naming tools this machine does not declare is usually the
    // default skill arriving beside somebody else's declaration. The engine
    // survives it, since the planner cannot resolve a tool that is not
    // declared, and the result is a machine that reports no capable tool for
    // everything and gives no hint why. Warned rather than refused, because a
    // partially matching skill is ordinary while a machine is being built.
    let strangers: Vec<&str> = skill
        .responses
        .iter()
        .filter(|r| declaration.tool(&r.tool).is_none())
        .map(|r| r.tool.0.as_str())
        .collect();
    if !strangers.is_empty() {
        eprintln!(
            "warning: the skill names {} this declaration does not have, including {}.\n\
             \x20        Pass --skill alongside --declaration if they belong together.",
            if strangers.len() == 1 { "a tool".to_string() } else { format!("{} tools", strangers.len()) },
            strangers[0]
        );
    }

    // ADI-P 6: a manufacturer's own knowledge. Optional, and absent means the
    // base profile alone, which is what every earlier version did.
    if let Some(path) = &knowledge_path {
        let extension: adi_protocol::knowledge::KnowledgeExtension =
            serde_yaml::from_str(&read(path)?)?;
        match domain.add_extension(extension, adi_protocol::knowledge::Layer::Vendor) {
            Ok(()) => println!("  knowledge    {}", path.display()),
            // 6.5 is explicit: a failing extension is rejected with a reason
            // and the engine continues on the knowledge it already has. A bad
            // extension degrades a machine to standard behaviour and never
            // stops it working, so this reports and carries on.
            Err(e) => eprintln!(
                "  knowledge    rejected: {e}\n\
                 \x20              continuing on base knowledge"
            ),
        }
    }

    // ADI-P 6.1's fourth layer: what this machine has learned about the person
    // using it. Stacked on top of the manufacturer's, which is what makes a
    // learned preference safe BY CONSTRUCTION rather than by a limit somebody
    // remembered to set.
    //
    // `ResolvedTargets::apply` intersects a layer with the one below and
    // refuses one that WIDENS a range. So a learned preference can only ever
    // narrow what the manufacturer declared:
    //
    //     manufacturer: a coffee is 90 to 130 g
    //     learned:      you take it 95 to 110    -> allowed
    //     learned:      you take it 140 to 160   -> refused, that is not a
    //                                               coffee any more
    //
    // Learning can tune a drink and cannot redefine one. Nothing had to be
    // built for that; it falls out of the layering the protocol already
    // specifies.
    //
    // Found by convention rather than by a flag, so a machine picks up its own
    // preferences with no extra argument: beside bob.yaml, look for
    // bob.user.yaml. Derived from the DECLARATION path because that is the one
    // argument always present.
    if let Some(user_path) = preferences_beside(&declaration_path) {
        match read(&user_path).and_then(|text| {
            serde_yaml::from_str::<adi_protocol::knowledge::KnowledgeExtension>(&text)
                .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })
        }) {
            Ok(extension) => {
                match domain.add_extension(extension, adi_protocol::knowledge::Layer::User) {
                    Ok(()) => println!("  learned      {}", user_path.display()),
                    // Reported and ignored, never fatal. A learned preference is
                    // a convenience; a machine that will not make coffee because
                    // its preferences file is malformed is worse than one that
                    // makes the manufacturer's coffee and says why.
                    //
                    // This is also where a preference that tried to widen a
                    // range lands, and the message is the useful part: it names
                    // what it could not apply.
                    Err(e) => eprintln!(
                        "  learned      rejected: {e}\n\
                         \x20              carrying on without it"
                    ),
                }
            }
            Err(e) => eprintln!(
                "  learned      {} could not be read: {e}",
                user_path.display()
            ),
        }
    }

    // Checked before anything is started. An argument that cannot be honoured
    // should be refused on its own terms, not after a failure to find a
    // binary that was never going to be used.
    let requested = parse_level(level.as_deref())?;

    // One link or the other, never both. A machine on a cable and a process on
    // this computer are different appliances, and quietly preferring one would
    // let someone debug a board while talking to the example build.
    if serial_path.is_some() && appliance_path.is_some() {
        return Err("give --serial or --appliance, not both: they are two \
                    different appliances"
            .into());
    }

    println!(
        "  machine      {} {}",
        declaration.device.vendor, declaration.device.model
    );

    // Before the link, not after. A local model takes tens of seconds to load
    // and a hosted one may have weights to fetch, and an appliance publishes
    // from the moment the port opens whether or not anybody is reading. Open
    // first and that reading goes into a buffer that fills and then drops
    // bytes, so the first request of the session is served from no state at
    // all while the engine was busy starting a model. It is also the ordinary
    // courtesy of not holding a link you cannot answer on.
    // One program for the interpreter and the chooser, where the
    // configuration names a process backend for the interpreter seam. The two
    // questions differ in shape and a backend tells them apart, which both
    // shipped backends do, so starting the program twice bought nothing and
    // cost a second set of a model's private buffers: measured at about
    // 2.1 GB on the recommended 4B model, which maps its weights and shares
    // those, but not the buffers beside them.
    let shared = shared_model_process(config_path.as_deref());

    let interpreter = load_interpreter(
        config_path.as_deref(),
        domain.as_ref(),
        &declaration,
        Some(&skill),
        shared.as_ref(),
    )?;
    let voice = Voice::load(config_path.as_deref())?;
    let explainer = load_explainer(config_path.as_deref(), shared.as_ref());
    let learner = load_learner(config_path.as_deref(), shared.as_ref());
    let chooser = load_chooser(config_path.as_deref(), requested, shared.as_ref())?;

    let transport: Box<dyn adi_session::Transport> = match &serial_path {
        Some(path) => {
            println!("  appliance    {path} at {baud} baud");
            // A port that opens is not evidence of a machine. Nothing is sent
            // here; `status` is what settles whether anything is listening.
            Box::new(SerialAppliance::open(path, baud)?)
        }
        None => {
            let appliance_path = appliance_path
                .map(|given| resolve_executable(&given))
                .unwrap_or_else(default_appliance);

            if !appliance_path.exists() {
                return Err(format!(
                    "no appliance at {}\n\nBuild it first:\n    cd sdk/c && make example\n\n\
                     Or reach a real board over a serial port:\n    \
                     adi run --serial {} --baud 115200",
                    appliance_path.display(),
                    if cfg!(windows) { "COM3" } else { "/dev/ttyUSB0" }
                )
                .into());
            }

            println!("  appliance    {}", appliance_path.display());
            Box::new(ChildAppliance::start(
                &appliance_path.to_string_lossy(),
                &[],
            )?)
        }
    };

    // ADI-P 3.3 makes D2 the default. It needs a chooser, so without one the
    // session opens at D0, which is deterministic and needs no model. The
    // level is reported either way rather than assumed, because a record
    // naming a level that did not run is the one thing 3.3 forbids. The
    // chooser was started above, with the other model seams.
    let mut session = match (requested, chooser) {
        (AutonomyLevel::D2, Some((chooser, label))) => {
            println!("  chooser      {label}");
            let selector = ModelActionSelector::new(chooser, &declaration)?;
            Session::open_with_selector(
                &declaration,
                Some(&skill),
                domain.as_ref(),
                Box::new(selector),
                transport,
            )
        }
        (AutonomyLevel::D1, Some((chooser, label))) => {
            println!("  chooser      {label}");
            Session::open_with_selector(
                &declaration,
                Some(&skill),
                domain.as_ref(),
                Box::new(ModelToolSelector { chooser }),
                transport,
            )
        }
        (AutonomyLevel::D0, _) => Session::open(
            &declaration,
            Some(&skill),
            domain.as_ref(),
            AutonomyLevel::D0,
            transport,
        )?,
        (wanted, None) => {
            // Printed and exited rather than returned, because a Box<dyn
            // Error> built from a String prints through Debug, which turns a
            // multi-line explanation into one line of escape sequences.
            eprintln!("
{} needs a chooser, and none is configured.
", wanted.as_str());
            eprintln!("Supply one with --config:");
            eprintln!("    cargo run -p adi-cli -- run --config models/models.example-process.yaml
");
            eprintln!("Or run deterministically with --level d0.");
            std::process::exit(2);
        }
    };

    println!("  autonomy     {}", session.level().as_str());

    println!();
    help();

    let mut now_ms: u64 = 0;

    // ADI-P 9.1 step 4 and a half: find out what the machine is set to before
    // taking an instruction about it. An appliance reports its setpoints on
    // first contact, and a request that arrives before the report cannot be
    // planned.
    session.synchronise(now_ms)?;

    // One thread reads input from here on, so the loop can also glance at the
    // keyboard while a drink is pouring instead of stopping to wait for it.
    let keyboard = Keyboard::start();

    loop {
        print!("> ");
        std::io::stdout().flush()?;

        let Some(line) = keyboard.next() else {
            break;
        };
        let line = line.as_str();
        now_ms += 60_000;

        // Every line, before anything is done with it, so what the machine
        // says next is recorded against the thing it is answering. Commands
        // like `status` and `do 3` are kept too: a person who typed one said
        // it, and a model trying to read the exchange would be puzzled by
        // replies to nothing.
        if !line.is_empty() && line != "quit" && line != "exit" {
            voice.heard(line);
        }

        // A failed command is reported and the session continues. Propagating
        // it ended the session on the first refusal, which is wrong twice over:
        // a refusal is frequently the correct outcome, and a person who has
        // just been refused is the one most likely to want to ask something
        // else. Only a broken link ends a session, and that is handled below.
        // Find out where the machine is before acting on what was just said.
        // The appliance reports its settings on contact and publishes its
        // readings as it takes them, and a person typing a request has
        // usually left a gap during which both arrived. Planning from what
        // was true when the session opened is planning from the past, and a
        // reading that arrived in that gap is exactly the one the request is
        // about. A link that has died is reported by the command that follows.
        let _ = session.tick(now_ms);

        // How many corrections the session had made before this command. The
        // difference afterwards is whether anything moved, which is the only
        // signal worth learning from.
        let before = session.history().len();

        let outcome = match line {
            "" => continue,
            "quit" | "exit" => break,
            "help" => {
                help();
                Ok(())
            }
            "status" => status(&mut session, now_ms),
            // The operator path with no model in it, which is how it is tested
            // on a bench and how it still works when the network is gone.
            "do" => {
                list_named(&session);
                Ok(())
            }
            // Only where what follows is a number.
            //
            // "do" opens an enormous number of English sentences, and treating
            // every one of them as this command ate the request: "do we have
            // any beans left" came back as "`do` takes a number". A command
            // namespace sharing a word with speech has to yield to the speech,
            // because the person did not know there was a command.
            other if other
                .strip_prefix("do ")
                .is_some_and(|rest| rest.trim().parse::<usize>().is_ok()) =>
            {
                let index = other
                    .strip_prefix("do ")
                    .and_then(|rest| rest.trim().parse::<usize>().ok())
                    .expect("the guard above parsed it");
                perform_named(
                    &mut session,
                    index,
                    now_ms,
                    &voice,
                    explainer.as_ref(),
                    &declaration_path,
                    &keyboard,
                    interpreter.as_ref(),
                )
            }
            "cycle" => cycle(
                &mut session,
                &declaration.domain_profile,
                now_ms,
                &voice,
                explainer.as_ref(),
            ),
            // A recorded utterance, since capturing from a microphone needs a
            // device and a platform library this tool does not carry.
            other if other.starts_with("listen ") => listen(
                &mut session,
                interpreter.as_ref(),
                &voice,
                other.trim_start_matches("listen ").trim(),
                now_ms,
                explainer.as_ref(),
                &declaration_path,
                &keyboard,
            ),
            _ => request(
                &mut session,
                interpreter.as_ref(),
                line,
                now_ms,
                &voice,
                explainer.as_ref(),
                &declaration_path,
                &keyboard,
            ),
        };

        let corrected_since = session.history().len().saturating_sub(before);

        // What has this machine learned?
        //
        // Asked only where something was CORRECTED. A drink nobody adjusted
        // teaches nothing, so there is nothing to ask about and asking anyway
        // would spend a model call on every coffee.
        //
        // After the outcome is known rather than before, and only on success:
        // a refused request is not a preference, it is a refusal.
        if outcome.is_ok() && corrected_since > 0 {
            remember(
                learner.as_ref(),
                &session,
                domain.as_ref(),
                &declaration_path,
                &voice,
            );
        }

        if let Err(error) = outcome {
            report(error.as_ref());

            // A session whose link is gone cannot do anything further, so
            // continuing would offer a prompt that can only fail.
            if is_fatal(error.as_ref()) {
                println!("
  the link to the appliance is gone, so the session ends here.");
                break;
            }
        }
    }

    println!("\nclosed.");
    Ok(())
}

/// The voice seams, where they are configured.
///
/// Held together because they are reached at the same points and a session with
/// one and not the other is not a useful configuration: a machine that hears and
/// cannot reply is worse than one that does neither.
struct Voice {
    /// Shareable for the same reason as `Loaded`: synthesis is the LONGEST leg
    /// of a spoken turn at about three and a half seconds, so it has to run off
    /// the keepalive loop too. Moving only the interpreting would have left
    /// more than half the appliance's tolerance still being spent on the loop.
    speaker: Option<std::sync::Arc<dyn Speaker + Send + Sync>>,
    /// Shareable as well, because a recording offered DURING an operation has
    /// to be transcribed on the worker. It is the shortest leg at about three
    /// quarters of a second, but it is still a network round trip and the
    /// keepalive has six seconds in total to play with.
    transcriber: Option<std::sync::Arc<dyn Transcriber + Send + Sync>>,
    /// Below this a transcript is treated as not heard, and the machine asks
    /// instead of acting. Carried here because it is read from the
    /// configuration and applied where a transcript is produced.
    min_confidence: f64,

    /// The last few exchanges, oldest first, for a model to read as prose.
    ///
    /// WHY IT LIVES ON THE VOICE. Everything the machine says to a person goes
    /// through `say`, whatever said it - an acknowledgement, a question, an
    /// answer, an explanation of what it just did. That makes this the one
    /// place that sees the machine's whole side of a conversation without
    /// every caller having to remember to report it.
    ///
    /// Capped at `TURNS_KEPT`. A `RefCell` because `say` is called through a
    /// shared reference from everywhere and threading a mutable one through
    /// would touch every function that speaks.
    turns: std::cell::RefCell<std::collections::VecDeque<adi_decision::Turn>>,

    /// How many things have been spoken, so each gets its own file.
    ///
    /// It was one fixed name, `spoken.wav`, rewritten every time. There are
    /// nine places that speak and a single turn uses several of them - the
    /// acknowledgement, then what happened - so each overwrote the one before
    /// it. Nothing failed and nothing was logged; a supervisor playing that
    /// file would simply have played whichever sentence won the race and
    /// silently lost the rest.
    spoken: std::cell::Cell<u64>,
}

/// How many exchanges are HELD, which is not how many are sent.
///
/// The configured `conversation` budget decides how many reach a model, the
/// same way `decisions` bounds the decision history: the engine keeps what it
/// has and the budget truncates on the way out. So this is only an upper bound
/// on memory, set well above any sensible budget, and raising the budget past
/// it is the one way to ask for more than is here.
const TURNS_HELD: usize = 32;

/// The default rate for each speech seam, used where the configuration states
/// none.
///
/// These are the rates the recommended models run at, and they are defaults
/// rather than requirements. Moonshine wants 16 kHz; Piper publishes voices at
/// both 16 and 22.05 kHz, so a configuration that cannot state its own rate
/// can only use half of them.
const DEFAULT_TRANSCRIBER_RATE: u32 = 16_000;
const DEFAULT_SPEAKER_RATE: u32 = 22_050;

/// The rate a speech seam runs at, as configured.
///
/// Audio is refused at any other rate rather than resampled, so this number
/// decides whether a backend works at all. A rate of zero would divide a
/// sample count by nothing when a duration is taken, so it is refused here
/// where it can still be explained.
fn speech_rate(seam: &SeamConfig, default: u32) -> Result<u32, String> {
    match seam.sample_rate {
        None => Ok(default),
        Some(0) => Err("sample_rate is 0, which is not a rate".into()),
        Some(rate) => Ok(rate),
    }
}

/// The context budget for one seam, as configured.
///
/// The third field in `SeamConfig` to be documented and read by nothing. The
/// same fault as the two below, and worse in its effect: a rate that is wrong
/// plays audio at the wrong pitch, which anyone notices, while a budget that
/// is wrong silently deletes part of the prompt.
///
/// It is not a soft limit. `ModelContext::enforce` drops whole sections to
/// meet it, styles among them, so a machine declaring enough tools and
/// variables to be useful had its drinks removed and the model was then asked
/// to name one. Nothing in the reply said why.
///
/// The default suits a small local model, which is what the SDK is built to
/// run. A hosted model holds a hundred times more and should be told so.
fn context_budget(seam: &SeamConfig) -> Result<ContextBudget, String> {
    let default = ContextBudget::default();

    // Applied first, so it survives the token branch below returning early.
    let default = match seam.decisions {
        None => default,
        Some(history) => ContextBudget { history, ..default },
    };

    // Zero is allowed here, unlike `context_tokens` below. No budget at all is
    // a contradiction; no conversation is a setting, and the one a kiosk wants.
    let default = match seam.conversation {
        None => default,
        Some(conversation) => ContextBudget {
            conversation,
            ..default
        },
    };

    match seam.context_tokens {
        None => Ok(default),
        // Refused rather than treated as "no limit", because a budget of zero
        // reads either way and the two are opposites. Nothing can be sent
        // within it, and `enforce` would drop every section it is allowed to
        // and still be over.
        Some(0) => Err("context_tokens is 0, which is not a budget".into()),
        Some(max_tokens) => Ok(ContextBudget { max_tokens, ..default }),
    }
}

/// The confidence threshold for a transcript, as configured.
///
/// A threshold outside 0 to 1 can never be met or can never be missed, and
/// either makes the check decorative. It is refused rather than clamped,
/// because a clamped threshold is one the integrator did not choose and would
/// have no way to notice.
fn transcript_threshold(seam: &SeamConfig) -> Result<f64, String> {
    match seam.min_confidence {
        None => Ok(adi_language::DEFAULT_MIN_TRANSCRIPT_CONFIDENCE),
        Some(value) if value.is_finite() && (0.0..=1.0).contains(&value) => Ok(value),
        Some(value) => Err(format!("min_confidence {value} is outside 0 to 1")),
    }
}

impl Voice {
    /// Start whichever of the two the configuration names.
    ///
    /// A seam that cannot be made ready is reported and left unserved, and the
    /// session continues without it. This follows the rule stated in
    /// `models.yaml`: an engine refuses the operations an unserved seam
    /// provides and names the seam, rather than failing. Before this, a
    /// speech program that would not start ended the session before the
    /// prompt, which also took away `cycle`, the one command that needs no
    /// model at all.
    fn load(config_path: Option<&Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let Some(path) = config_path else {
            return Ok(Self {
                speaker: None,
                transcriber: None,
                turns: std::cell::RefCell::new(std::collections::VecDeque::new()),
            spoken: std::cell::Cell::new(0),
                min_confidence: adi_language::DEFAULT_MIN_TRANSCRIPT_CONFIDENCE,
            });
        };

        let config = ModelsConfig::load(path)?;

        let speaker = {
            let seam = config.seam(Seam::Speaker);
            match (seam.backend, seam.command.clone()) {
                (Backend::Process, Some(command)) => {
                    match speech_rate(&seam, DEFAULT_SPEAKER_RATE)
                        .map_err(|e| e.to_string())
                        .and_then(|rate| {
                            ProcessSpeaker::start(&command, &seam.args, rate)
                                .map(|s| (s, rate))
                                .map_err(|e| e.to_string())
                        }) {
                        Ok((speaker, rate)) => {
                            println!("  speaker      process ({command}) at {rate} Hz");
                            Some(std::sync::Arc::new(speaker) as std::sync::Arc<dyn Speaker + Send + Sync>)
                        }
                        Err(e) => {
                            println!("  speaker      unavailable: {e}");
                            None
                        }
                    }
                }
                _ => None,
            }
        };

        let mut min_confidence = adi_language::DEFAULT_MIN_TRANSCRIPT_CONFIDENCE;
        let transcriber = {
            let seam = config.seam(Seam::Transcriber);
            match (seam.backend, seam.command.clone()) {
                (Backend::Process, Some(command)) => {
                    match transcript_threshold(&seam).and_then(|threshold| {
                        let rate = speech_rate(&seam, DEFAULT_TRANSCRIBER_RATE)?;
                        ProcessTranscriber::start(&command, &seam.args, rate)
                            .map(|t| (t, rate, threshold))
                            .map_err(|e| e.to_string())
                    }) {
                        Ok((transcriber, rate, threshold)) => {
                            min_confidence = threshold;
                            println!(
                                "  transcriber  process ({command}) at {rate} Hz, asking below {threshold}"
                            );
                            Some(std::sync::Arc::new(transcriber)
                                as std::sync::Arc<dyn Transcriber + Send + Sync>)
                        }
                        Err(e) => {
                            println!("  transcriber  unavailable: {e}");
                            None
                        }
                    }
                }
                _ => None,
            }
        };

        Ok(Self {
            speaker,
            transcriber,
            turns: std::cell::RefCell::new(std::collections::VecDeque::new()),
            spoken: std::cell::Cell::new(0),
            min_confidence,
        })
    }

    /// Say something, where a speaker is configured.
    ///
    /// The audio is written beside the working directory rather than played.
    /// Playback needs an output device and a platform library, which is a
    /// packaging concern; what matters to the protocol is that the text reached
    /// the seam and came back as samples.
    /// A person has said something: a new exchange begins.
    fn heard(&self, said: &str) {
        let mut turns = self.turns.borrow_mut();
        while turns.len() >= TURNS_HELD {
            turns.pop_front();
        }
        turns.push_back(adi_decision::Turn {
            said: said.to_string(),
            replies: Vec::new(),
        });
    }

    /// The exchanges so far, oldest first.
    ///
    /// Cloned rather than borrowed, because the caller holds this across the
    /// call to a model and `say` must stay free to record what comes back.
    fn recent(&self) -> Vec<adi_decision::Turn> {
        self.turns.borrow().iter().cloned().collect()
    }

    /// Say something that has ALREADY been synthesised.
    ///
    /// The counterpart to `say` for anything answered from the worker thread,
    /// where the three and a half seconds of synthesis has been spent off the
    /// keepalive loop. Calling `say` here would spend it again, on the loop,
    /// which is the whole thing this avoids.
    /// Where the next thing spoken will be written.
    ///
    /// Numbered from one and never reused within a run. A supervisor plays
    /// them in order and deletes each when it has, which is what makes a
    /// sentence lost rather than overwritten if it falls behind.
    fn write_spoken(&self, audio: &adi_language::Audio) {
        let nth = self.spoken.get() + 1;
        self.spoken.set(nth);
        let path = std::path::PathBuf::from(format!("spoken-{nth:04}.wav"));
        match write_wav(&path, audio) {
            Ok(()) => println!(
                "  spoken       {} samples at {} Hz, written to {}",
                audio.samples.len(),
                audio.sample_rate,
                path.display()
            ),
            Err(e) => println!("  speaker      could not write the audio: {e}"),
        }
    }

    fn played(&self, text: &str, audio: Option<adi_language::Audio>) {
        if let Some(turn) = self.turns.borrow_mut().back_mut() {
            turn.replies.push(text.to_string());
        }
        match audio {
            Some(audio) => self.write_spoken(&audio),
            // No speaker configured, or synthesis failed on the far side. The
            // text has been printed, so nothing is lost but the voice.
            None => {}
        }
    }

    fn say(&self, text: &str) {
        // Recorded BEFORE the speaker is consulted. `say` returns early with
        // no speaker configured, which is every build until the voice work
        // lands - and Bob's own `speaker: backend: none` today. Recording
        // inside the speaking would have captured nothing on exactly the
        // machine this is for, and the conversation would have looked empty
        // for a reason nothing in the output would have explained.
        if let Some(turn) = self.turns.borrow_mut().back_mut() {
            turn.replies.push(text.to_string());
        }

        let Some(speaker) = &self.speaker else {
            return;
        };

        match speaker.speak(text) {
            Ok(audio) => self.write_spoken(&audio),
            // A machine that cannot speak should still work. The text was
            // already printed, so nothing is lost but the voice.
            Err(e) => println!("  speaker      unavailable: {e}"),
        }
    }
}

/// A minimal 16-bit mono WAV, so what the speaker produced can be listened to.
///
/// Hand written rather than taken from a crate: it is forty lines, it is the
/// only audio file this tool writes, and a dependency for it would be carried by
/// every build of the SDK.
fn write_wav(path: &PathBuf, audio: &adi_language::Audio) -> std::io::Result<()> {
    use std::io::Write as _;

    let data_bytes = (audio.samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM header size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM, uncompressed
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&audio.sample_rate.to_le_bytes());
    out.extend_from_slice(&(audio.sample_rate * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in &audio.samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }

    std::fs::File::create(path)?.write_all(&out)
}

/// Read a 16-bit mono WAV, so a recorded utterance can be transcribed.
///
/// Capturing from a microphone needs a platform library and a device, which this
/// tool deliberately does not carry. A file is enough to exercise the seam.
fn read_wav(path: &Path) -> Result<adi_language::Audio, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("{} is not a WAV file", path.display()).into());
    }

    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    let bits = u16::from_le_bytes([bytes[34], bytes[35]]);

    if channels != 1 || bits != 16 {
        return Err(format!(
            "{} is {channels} channel at {bits} bit; 1 channel at 16 bit is required",
            path.display()
        )
        .into());
    }

    // Find the data chunk rather than assuming it follows the format chunk,
    // since a WAV may carry others between them.
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size =
            u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
                as usize;
        if id == b"data" {
            let end = (at + 8 + size).min(bytes.len());
            let samples = bytes[at + 8..end]
                .chunks_exact(2)
                .map(|p| i16::from_le_bytes([p[0], p[1]]))
                .collect();
            return Ok(adi_language::Audio::new(samples, sample_rate));
        }
        at += 8 + size + (size & 1);
    }

    Err(format!("{} carries no data chunk", path.display()).into())
}

/// Say what went wrong in terms a person can act on.
///
/// The underlying errors are precise and are written for a developer reading a
/// log. A person at a prompt needs to know whether to try something else, and
/// the raw Debug form of a Rust enum does not tell them.
/// What an appliance's refusal code means, for somebody standing at a prompt.
///
/// The codes are ADI-P 9.2 and are written for a log. Each of these has a
/// different thing to do about it, which is the only reason to translate them.
fn plainly(reason: &str) -> &str {
    match reason {
        "Interlock" => {
            "a door, guard or cover is open. The appliance enforces this itself, so nothing the engine sends will get around it"
        }
        "Busy" => "the machine is already doing something. Try again when it has finished",
        "RateLimited" => {
            "too soon after the last call of that tool. The declaration states the interval"
        }
        "PreconditionFailed" => {
            "a condition that tool requires does not hold. The declaration lists them under the tool"
        }
        "EnvelopeViolation" | "InvariantViolation" => {
            "that would breach a limit the appliance enforces itself, so none of it ran"
        }
        "ResourceExhausted" => "something the tool consumes has run out",
        "Fault" => "the machine is faulted and will not act until that is cleared",
        "NotAuthorised" => "the machine does not permit the engine to use that tool",
        "UnknownTool" => {
            "the appliance has no such tool, so it and the engine are working from different declarations"
        }
        "OutOfRange" | "UnitMismatch" => {
            "the appliance rejected the value itself, which means the engine sent something it should have caught. Worth reporting"
        }
        "Malformed" => "the appliance could not decode the message",
        other => other,
    }
}

fn report(error: &(dyn std::error::Error + 'static)) {
    let text = error.to_string();

    // The refusals worth recognising, because each has a different thing to do
    // about it. Matched on the message rather than the type so that this stays
    // in the CLI and the session keeps its error vocabulary to itself.
    if text.contains("no candidate") || text.contains("NoCandidate") {
        println!("  cannot        nothing this machine declares can move that.");
        println!("                `adi capabilities` lists what it can change.");
    } else if text.contains("envelope") {
        println!("  refused       that would breach a safety limit, so none of it ran.");
        println!("                {text}");
    } else if let Some(reason) = text.strip_prefix("the appliance stopped the operation: ") {
        // It started and was stopped, which is not the same as never having
        // run. Somebody opening a door mid-cycle is the ordinary case.
        println!("  stopped       by the appliance, part way through: {}", plainly(reason));
    } else if let Some(reason) = text.strip_prefix("the appliance refused the call: ") {
        // The appliance's own refusal, in words. It said why in a code, and
        // printing the code inside a sentence that already says the appliance
        // refused tells a person nothing they can act on.
        println!("  refused       by the appliance: {}", plainly(reason));
    } else if text.contains("both move") || text.contains("one iteration") {
        println!("  refused       the plan changed one thing two ways, which would make");
        println!("                the result impossible to attribute. Nothing ran.");
        // AND WHAT THEY WERE. The error underneath names both steps and the
        // variable - "steps 'x' and 'y' both move 'z' in one iteration" - and
        // this printed a friendlier sentence INSTEAD of it, throwing away the
        // only thing anybody could act on. It took a twelve-run sweep to find
        // out what was conflicting, which is a diagnosis the message should
        // simply have handed over.
        println!("                {text}");
    } else {
        println!("  failed        {text}");
    }
}

/// Whether the session can usefully continue.
/// Where a machine's learned preferences live, if it has any.
///
/// Beside the declaration, named after it: `bob.yaml` -> `bob.user.yaml`.
///
/// A convention rather than a flag, because preferences are not something a
/// person should have to remember to pass. A machine that only honours what it
/// has learned when told to look is a machine that has not learned anything.
///
/// Returns None where there is no such file, which is the ordinary case and
/// not an error: a machine that has learned nothing runs on the
/// manufacturer's knowledge, exactly as it did before this existed.
fn preferences_beside(declaration: &Path) -> Option<PathBuf> {
    // `bob.yaml` -> `bob`, and `bob.knowledge.yaml` -> `bob` too, so passing
    // either one finds the same file rather than looking for
    // `bob.knowledge.user.yaml`.
    let stem = declaration.file_stem()?.to_str()?;
    let stem = stem.strip_suffix(".knowledge").unwrap_or(stem);

    let candidate = declaration.with_file_name(format!("{stem}.user.yaml"));
    candidate.exists().then_some(candidate)
}

fn is_fatal(error: &(dyn std::error::Error + 'static)) -> bool {
    let text = error.to_string();
    text.contains("the link failed") || text.contains("went quiet")
}

fn help() {
    println!("  cycle    feed a finished cycle and let the engine correct it");
    println!("           this needs no model and exercises the whole loop");
    println!("  status   send a heartbeat and report what the appliance says
  listen <file.wav>
           transcribe a recorded utterance and treat it as a request
           16 bit mono. Needs a transcriber; see `adi models check`");
    println!("  do       list the actions you can ask for by name");
    println!("  do <n>   carry one out. Needs no model, so it works on a bench");
    println!("  quit     end the session");
    println!();
    println!("  anything else is treated as a spoken request, which needs an");
    println!("  interpreter. See `adi models check`.");
    println!();
}

/// The observation path: a cycle finished, something was wrong with it, and
/// the engine corrects the machine. No model is involved at any point.
///
/// The cycle below is espresso, and deliberately so: it is the worked example
/// the documents walk through. On any other profile it would be a coffee shot
/// shown to a machine that is not a coffee machine, which teaches the reader
/// something false about what they are running. So it is offered only where it
/// means something, and elsewhere says what it would take to have one.
/// Typed input, readable without stopping to wait for it.
///
/// `read_line` blocks, and during a drink the loop has to be able to look at
/// the keyboard and carry on if there is nothing there. So one thread does
/// nothing but read lines and hand them over.
///
/// It is also where the microphone will arrive: `bob_voice.py` writes a line
/// per utterance, and this is the queue it writes into. Nothing above here has
/// to know which of the two it was.
struct Keyboard {
    lines: std::sync::mpsc::Receiver<String>,

    /// One line read too early, waiting for the prompt to come round.
    ///
    /// Something said during a drink cannot be acted on then - a second order
    /// would queue behind a drink nobody can see the end of - but it must not
    /// be lost either. It is held here and taken first next time.
    kept: std::cell::RefCell<Option<String>>,
}

impl Keyboard {
    fn start() -> Self {
        let (send, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            loop {
                let mut line = String::new();
                match stdin.lock().read_line(&mut line) {
                    // End of input, or a console that has gone away. The
                    // channel closing is how the loop above learns of it.
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if send.send(line.trim().to_string()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            lines,
            kept: std::cell::RefCell::new(None),
        }
    }

    /// Wait for a line. None once input has ended.
    fn next(&self) -> Option<String> {
        if let Some(kept) = self.kept.borrow_mut().take() {
            return Some(kept);
        }
        self.lines.recv().ok()
    }

    /// Put a line back, to be taken at the next prompt.
    fn keep(&self, line: &str) {
        *self.kept.borrow_mut() = Some(line.to_string());
    }

    /// Whatever is already there, without waiting.
    fn ready(&self) -> Option<String> {
        self.lines.try_recv().ok()
    }
}

/// Wait for an operation to finish, listening while it runs.
///
/// WHY THIS EXISTS. `request` and `act` do not return until the appliance
/// reports the operation COMPLETE, which for a coffee is most of a minute, and
/// the loop was inside that call for the whole time. So nothing read the
/// keyboard: during a drink Bob was not slow, he was deaf. "Stop" could not be
/// said, although the tool is declared and the firmware implements it.
///
/// The ticking is what must not stop. It carries the heartbeat, and an
/// appliance that hears nothing for its `control_timeout_ms` - six seconds for
/// Bob - concludes its controller has died and abandons the drink. So nothing
/// slow belongs in this loop: a stop is a word to match, not a question to
/// think about.
/// Whether a line is an instruction to this tool rather than something said.
///
/// Kept deliberately small and literal. It exists to stop a COMMAND being sent
/// to a language model, not to guess at intent - anything it does not
/// recognise is treated as speech, which is the right way round: a sentence
/// mistaken for a command would be refused, while a command mistaken for a
/// sentence is merely understood and then waits.
fn is_command(line: &str) -> bool {
    let line = line.trim();
    // `listen <file>` is deliberately NOT here. It looks like a command and is
    // one at the prompt, but what it carries is speech, and it is the only way
    // a voice supervisor can say anything at all. Deferring it made a spoken
    // question mid-brew impossible for the one caller that needs it.
    matches!(line, "status" | "cycle" | "quit" | "exit" | "do")
        || line.starts_with("do ")
}

/// Act on something understood WHILE an operation is still running.
///
/// The division is not about what is convenient, it is about what touches the
/// appliance. Answering a question reads state the engine already holds and
/// sends nothing, so it happens now. Anything that would move the machine has
/// to wait for the mechanism, which is busy - so it is handed back to the
/// prompt rather than refused, because the person has not asked for anything
/// unreasonable, only something that cannot happen yet.
fn answer_now(
    session: &mut Session<'_>,
    voice: &Voice,
    keyboard: &Keyboard,
    thought: Thought,
    at_ms: u64,
    // What was actually said, so that something which has to wait can be put
    // back as the person's own words. The model's offered sentence is not a
    // substitute: handing "Coming right up!" to the prompt would have the
    // machine trying to carry out its own acknowledgement.
    about: &str,
) {
    match thought.understood {
        Err(reason) => println!("  could not    {reason}"),

        // Touches nothing. Given straight away, with the audio the worker
        // already synthesised.
        Ok(Understood::Answer { text, drew_on }) => {
            println!("  answers      {text}");
            if !drew_on.is_empty() {
                println!("  from         {}", drew_on.join(", "));
            }
            voice.played(&text, thought.audio);
        }

        // Also touches nothing: it is a question back.
        Ok(Understood::Clarify { question, because }) => {
            println!("  asks         {question}");
            println!("  because      {because}");
            voice.say(&question);
        }

        // A stop is the one moving thing that must not wait - it is the whole
        // point of being able to speak during a drink.
        Ok(Understood::Action(index)) if stop_action(session) == Some(index) => {
            println!("  stopping     ...");
            match session.act(index, at_ms) {
                Ok(_) => println!("  stopped      it did not finish"),
                Err(error) => println!("  could not    {error}"),
            }
        }

        // Everything else moves the machine, and the machine is busy. Handed
        // to the prompt, which takes it the moment this operation ends.
        Ok(_) => {
            println!("  after this   that needs the machine, so it waits its turn");
            keyboard.keep(about);
        }
    }
}

/// An utterance being understood on another thread, while the loop ticks.
///
/// WHY THIS EXISTS. Understanding what somebody said takes five to seven
/// seconds all told: about three quarters of a second to transcribe, one to
/// three to interpret, and three and a half to synthesise the reply - the last
/// of those measured twice against the live provider. Bob declares a
/// `control_timeout_ms` of six seconds, and the loop that waits out a drink is
/// the same loop that sends the keepalive. Doing any of that work on the loop
/// stops the heartbeat for longer than the appliance tolerates, it declares
/// its controller dead, and it abandons the drink. Somebody asking "how long
/// is left?" would have destroyed the coffee they were asking about.
///
/// So the thinking happens here and NOTHING here touches the appliance. The
/// snapshot is owned data, the answer comes back through a channel, and the
/// main thread is the only thing that ever sends. One door, still.
struct Thinking {
    answer: std::sync::mpsc::Receiver<Thought>,

    /// Kept so the utterance can be named while it is still being thought
    /// about, and reported if the thread dies without answering.
    about: String,
}

/// Everything the model needs, owned, so it can cross a thread boundary.
///
/// `SelectionContext` borrows, which is right for the engine and no use here,
/// so each part is cloned once. All of it is small: a few decision records, a
/// handful of readings and at most a few exchanges. It is a snapshot by
/// design, not by accident - taken the moment somebody speaks, so the answer
/// describes the machine as it was asked about rather than as it became.
struct Snapshot {
    /// What was said, where it arrived as text.
    utterance: String,

    /// What was said, where it arrived as sound.
    ///
    /// Transcribing on the worker rather than before it is the difference
    /// between a spoken question mid-brew working and not working. The only
    /// way a supervisor can hand speech to this tool is `listen <file>`, so
    /// treating that as a command to defer - which is right for every OTHER
    /// command - left the feature unreachable by the only thing that uses it.
    heard: Option<adi_language::Audio>,

    /// Below this a transcript is a guess, and the engine asks rather than
    /// acting on it. Carried because the worker is the one that now decides.
    min_confidence: f64,
    history: Vec<adi_core::record::DecisionRecord>,
    memory: adi_core::memory::RecentCycles,
    measured: std::collections::BTreeMap<adi_protocol::ids::VariableId, Quantity>,
    recent: Vec<adi_decision::Turn>,
}

/// What came back, with the speaking already done.
struct Thought {
    /// `Err` where the model or the network failed. Reported, never guessed at.
    understood: Result<Understood, String>,

    /// Already synthesised, because synthesis is the longest leg and belongs
    /// on this side of the channel. The main thread only plays it.
    audio: Option<adi_language::Audio>,
}

/// Think about an utterance without the appliance noticing.
///
/// Spawns and returns at once. The loop carries on ticking and picks the
/// answer up with [`Thinking::ready`] when it arrives.
fn think(
    interpreter: std::sync::Arc<dyn Interpreter + Send + Sync>,
    speaker: Option<std::sync::Arc<dyn Speaker + Send + Sync>>,
    transcriber: Option<std::sync::Arc<dyn Transcriber + Send + Sync>>,
    snapshot: Snapshot,
) -> Thinking {
    let (tell, answer) = std::sync::mpsc::channel();
    let about = snapshot.utterance.clone();

    std::thread::spawn(move || {
        let context = SelectionContext {
            history: &snapshot.history,
            cycle: None,
            memory: Some(&snapshot.memory),
            rejected: None,
            settings: None,
            measured: Some(&snapshot.measured),
            recent: &snapshot.recent,
            // No style yet. These contexts serve the INTERPRETER, and the
            // style is what the interpreter works out - it cannot be an input
            // to the call that produces it.
            style: None,
};

        // Sound first, where it arrived as sound.
        let mut words = snapshot.utterance.clone();
        if let Some(audio) = &snapshot.heard {
            let Some(transcriber) = &transcriber else {
                let _ = tell.send(Thought {
                    understood: Err(
                        "no transcriber is configured, so audio cannot be understood"
                            .to_string(),
                    ),
                    audio: None,
                });
                return;
            };
            match transcriber.transcribe(audio) {
                Ok(transcript) if transcript.confidence < snapshot.min_confidence => {
                    // Not refused and not guessed at. ADI-P treats a transcript
                    // below the threshold as something to ask about, and asking
                    // touches nothing, so it can be asked mid-operation.
                    let _ = tell.send(Thought {
                        understood: Ok(Understood::Clarify {
                            question: "Sorry, could you say that again?".to_string(),
                            because: format!(
                                "that came through at {:.2}, below the {:.2} this                                  machine acts on",
                                transcript.confidence, snapshot.min_confidence
                            ),
                        }),
                        audio: None,
                    });
                    return;
                }
                Ok(transcript) => words = transcript.text,
                Err(e) => {
                    let _ = tell.send(Thought {
                        understood: Err(e.to_string()),
                        audio: None,
                    });
                    return;
                }
            }
        }

        let reply = interpreter.reply(&words, &context);

        // The model's offered sentence is deliberately dropped.
        //
        // It is an acknowledgement meant to be spoken WHILE something is
        // carried out - "coming right up". Nothing understood here is carried
        // out here: an answer is its own words, and an order has to wait for
        // the mechanism, so speaking "coming right up" would announce a drink
        // that is not being made. When the prompt takes that order up the turn
        // runs properly and the acknowledgement is spoken then, by the code
        // that is actually doing it.
        let understood = match reply {
            Ok(reply) => Ok(reply.understood),
            Err(e) => Err(e.to_string()),
        };

        // Synthesised here, on this thread, for the same reason the
        // interpreting is: it is the longest leg. Only what will actually be
        // said aloud, and only where it is an answer - a step the main thread
        // has yet to take may still be refused, and speaking first would have
        // Bob announcing things that then do not happen.
        let audio = match (&understood, &speaker) {
            (Ok(Understood::Answer { text, .. }), Some(speaker)) => {
                speaker.speak(text).ok()
            }
            _ => None,
        };

        // A closed channel means the main thread gave up waiting, which is not
        // this thread's business to report.
        let _ = tell.send(Thought { understood, audio });
    });

    Thinking { answer, about }
}

impl Thinking {
    /// The answer, if it has arrived. Never blocks.
    fn ready(&self) -> Option<Thought> {
        match self.answer.try_recv() {
            Ok(thought) => Some(thought),
            // Disconnected means the thread ended without sending, which only
            // happens if it panicked. Reported as a failure rather than left
            // to wait forever.
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Thought {
                understood: Err(format!(
                    "the language service stopped without answering \"{}\"",
                    self.about
                )),
                audio: None,
            }),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        }
    }
}

/// Take a snapshot of what the model will need.
fn snapshot(session: &Session<'_>, voice: &Voice, utterance: &str) -> Snapshot {
    Snapshot {
        utterance: utterance.to_string(),
        heard: None,
        min_confidence: voice.min_confidence,
        history: session.history().to_vec(),
        memory: session.memory().clone(),
        measured: session.measurements().clone(),
        recent: voice.recent(),
    }
}

/// What became of the operation that was being waited on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attended {
    /// It ran to completion, or there was nothing running to wait for.
    Finished,
    /// It was cut short, so whatever was asked for did not happen.
    ///
    /// The caller MUST NOT then report the operation as carried out. It did
    /// that: a stopped cappuccino was announced as "I prepared a cappuccino as
    /// requested", because the report is written from the record - which says
    /// what was intended - and nothing told it the outcome.
    Stopped,
}

fn attend(
    session: &mut Session<'_>,
    keyboard: &Keyboard,
    now_ms: u64,
    voice: &Voice,
    // Passed down so that something said while an operation runs can be
    // UNDERSTOOD rather than only matched against the word "stop". The
    // understanding happens on another thread; see `think`.
    mind: Option<&(std::sync::Arc<dyn Interpreter + Send + Sync>, String)>,
) -> Result<Attended, Box<dyn std::error::Error>> {
    if !session.working() {
        return Ok(Attended::Finished);
    }

    let mut at_ms = now_ms;
    let mut said = false;

    // At most one utterance is being thought about at a time. A second one
    // arriving while the first is still out is kept for the prompt: two
    // answers landing together would talk over each other, and the machine is
    // in the middle of something either way.
    let mut thinking: Option<Thinking> = None;

    while session.working() {
        session.tick(at_ms)?;
        at_ms = at_ms.saturating_add(250);

        // Collected before the appliance is looked at, so an answer is given
        // the moment it is ready rather than one tick later.
        if let Some(pending) = &thinking {
            if let Some(thought) = pending.ready() {
                let about = pending.about.clone();
                thinking = None;
                answer_now(session, voice, keyboard, thought, at_ms, &about);
            }
        }

        if let Some(ended) = session.collect() {
            return match ended {
                adi_session::Ended::Completed => Ok(Attended::Finished),
                adi_session::Ended::Aborted(_) => {
                    println!("  stopped      part way through");
                    Ok(Attended::Stopped)
                }
                adi_session::Ended::Refused(reason) => {
                    println!("  refused      {reason:?} after it had started");
                    Ok(Attended::Stopped)
                }
            };
        }

        match keyboard.ready().as_deref() {
            Some("stop") | Some("cancel") => {
                // Through the front door. The stop is an ordinary named action
                // by index, so it goes through the envelope and writes its own
                // record like anything else - which is the whole reason the
                // waiting was moved out here rather than given a callback.
                match stop_action(session) {
                    Some(index) => {
                        println!("  stopping     ...");
                        match session.act(index, at_ms) {
                            Ok(_) => {
                                println!("  stopped      it did not finish");
                                return Ok(Attended::Stopped);
                            }
                            Err(error) => {
                                // A machine that will not stop says so. Better
                                // than reporting a drink ended while the
                                // mechanism runs on.
                                println!("  could not    {error}");
                            }
                        }
                    }
                    None => println!("  no stop      this machine declares no way to stop"),
                }
            }
            Some("") | None => {}
            Some(other) => {
                // Understood on another thread, so the keepalive keeps going.
                //
                // This used to keep the line for the prompt and say "one
                // moment" - which was the only safe thing to do when every
                // model call blocked this loop. It is not a limitation worth
                // keeping: an answer touches nothing. "How long is left?" or
                // "is there enough syrup?" reads state the engine already
                // holds, sends nothing, and can be given while the drink
                // pours. Only a new ORDER has to wait, and it waits because
                // the mechanism is busy rather than because the engine cannot
                // think.
                // A COMMAND IS NOT SPEECH. `listen <file>`, `do 3`, `status`
                // and the rest are instructions to this tool, and handing one
                // to the interpreter asks a model what somebody meant by
                // "listen C:/tmp/order.wav" - which is nothing. It produced
                // exactly that the first time the supervisor sent a second
                // recording while a drink was pouring.
                //
                // They are kept for the prompt rather than run here for the
                // same reason an order is: each of them moves the machine or
                // starts a turn of its own, and the mechanism is busy.
                if is_command(other) {
                    keyboard.keep(other);
                    if !said {
                        println!("  after this   \"{other}\" waits its turn");
                        said = true;
                    }
                    continue;
                }

                if let (Some((interpreter, _)), None) = (mind, &thinking) {
                    // A recording, or words typed at the prompt. Either way
                    // the whole turn goes to the worker from here.
                    let (mut shot, what) = match other.strip_prefix("listen ") {
                        Some(path) => match read_wav(Path::new(path.trim())) {
                            Ok(audio) => {
                                let mut shot = snapshot(session, voice, "");
                                shot.heard = Some(audio);
                                (shot, "a recording".to_string())
                            }
                            Err(e) => {
                                println!("  could not    read {}: {e}", path.trim());
                                continue;
                            }
                        },
                        None => (
                            snapshot(session, voice, other),
                            format!("\"{other}\""),
                        ),
                    };

                    // Only typed words are recorded as an exchange here. A
                    // recording has no text until the worker has transcribed
                    // it, and guessing at one would put the file path into the
                    // conversation as though somebody had said it.
                    if shot.heard.is_none() {
                        voice.heard(other);
                    } else {
                        shot.utterance = String::new();
                    }

                    thinking = Some(think(
                        interpreter.clone(),
                        voice.speaker.clone(),
                        voice.transcriber.clone(),
                        shot,
                    ));
                    println!("  thinking     {what}");
                    continue;
                }

                // No interpreter, or already thinking about something else.
                // Kept, not dropped: this once said "one moment" and threw the
                // line away, so a person was told they had been heard and then
                // ignored. It ate four cases of its own acceptance test before
                // anybody noticed.
                keyboard.keep(other);
                // Anything else has to wait. A second order would queue behind
                // a drink nobody can see the end of, and asking the model what
                // this meant would pause the heartbeat.
                if !said {
                    println!("  busy         one moment - \"{other}\" after this finishes");
                    said = true;
                }
                // NOT a break. Keeping the line is right; stopping the loop
                // is not - this loop carries the heartbeat, and an appliance
                // that hears nothing for its control_timeout_ms abandons what
                // it is doing. Leaving early would drop the drink in order to
                // be polite about a sentence.
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    Ok(Attended::Finished)
}

/// The index of this machine's stop, among the actions an operator may name.
///
/// By shape rather than by identifier: `adi.espresso.brew.stop` is one
/// domain's name for it, and this is the generic runner. None where the
/// machine declares no such thing, and a machine with no stop cannot be
/// interrupted - which is worth saying plainly rather than inventing a tool it
/// does not have.
fn stop_action(session: &Session<'_>) -> Option<usize> {
    session
        .named_actions()
        .iter()
        .position(|action| action.tool.0.ends_with(".stop"))
}

fn cycle(
    session: &mut Session<'_>,
    profile: &str,
    now_ms: u64,
    voice: &Voice,
    explainer: &dyn Explainer,
) -> Result<(), Box<dyn std::error::Error>> {
    if profile != adi_domain_espresso::registry::PROFILE_ID {
        println!("  no example      this command replays a worked espresso cycle, and");
        println!("                  this machine runs the {profile} profile.");
        println!("                  An appliance reports its own cycles over the link;");
        println!("                  this is demonstration data, not part of the protocol.");
        return Ok(());
    }

    let n = session.memory().len() + 1;

    // A short shot, which the profile diagnoses as under extracted.
    let mut settings = std::collections::BTreeMap::new();
    settings.insert(
        VariableId::from("grind_setting"),
        Quantity::new(24.0, Unit::Step),
    );
    settings.insert(VariableId::from("dose_mass"), Quantity::new(18.0, Unit::G));

    let observed = Cycle {
        cycle_id: format!("c{n}"),
        subject_id: None,
        operator_id: None,
        settings,
        measured: std::collections::BTreeMap::new(),
        traces: vec![adi_core::Trace {
            signal: adi_protocol::SemanticId::from("adi.espresso.signal.mass"),
            // Flow begins at 4 s and stops at 22 s, so an 18 second shot
            // against a 25 to 32 second target.
            samples: vec![
                (0.0, 0.0),
                (4.0, 1.0),
                (10.0, 9.0),
                (16.0, 21.0),
                (22.0, 32.0),
            ],
            unit: Unit::G,
            accuracy_abs: 0.2,
            filtered: false,
        }],
        events: Vec::new(),
        feedback: SubjectiveFeedback {
            descriptors: vec!["sour".into()],
            rating: None,
            note: None,
        },
    };

    println!("  observing    an 18 s shot, described as sour");

    let turn = session.observe(observed, "adi.espresso.style.espresso", None, now_ms)?;

    if turn.nominal {
        println!("  result       within target, nothing to do");
        return Ok(());
    }

    for (record, invocation) in turn.records.iter().zip(&turn.performed) {
        println!("  diagnosed    {}", record.cause);
        println!("  chose        {}", invocation.tool.as_str());
        for (name, value) in &invocation.numeric_arguments {
            println!("  sent         {name} = {} {:?}", value.value, value.unit);
        }
        println!("  accepted     by the appliance");

        // A correction held at the end of its range is partial, and saying so
        // is the difference between a person trying something else and a person
        // concluding the machine does not work.
        for (parameter, wanted) in &record.clamped_from {
            println!(
                "  at limit     {parameter} could not reach {wanted}; this correction is partial"
            );
        }

        // The same sentence the request path says, from the same seam. Two
        // copies of this drifted once already.
        say_what_happened(explainer, record, voice);
    }

    Ok(())
}

fn status(session: &mut Session<'_>, now_ms: u64) -> Result<(), Box<dyn std::error::Error>> {
    session.tick(now_ms)?;

    // What the heartbeat actually produced, rather than the fact that it was
    // sent. This printed "alive" unconditionally, including against a port
    // with nothing behind it, which is the one question a person runs this
    // command to answer.
    if session.answered_last_tick() {
        println!("  link         alive, the appliance answered");
    } else {
        println!("  link         open, and nothing answered");
        println!("               a port that opens is not evidence of a machine.");
        println!("               check power, wiring, baud rate, and that the");
        println!("               firmware calls adip_poll every loop.");
    }

    println!("  cycles held  {}", session.memory().len());
    println!("  decisions    {}", session.history().len());

    // What the machine says about itself. Empty until it reports, which a
    // conforming appliance does on contact.
    for (variable, value) in session.measurements() {
        println!("  measured     {} = {} {:?}", variable.0, value.value, value.unit);
    }

    Ok(())
}

/// The request path: a person asked for something in words.
fn listen(
    session: &mut Session<'_>,
    interpreter: Option<&(std::sync::Arc<dyn Interpreter + Send + Sync>, String)>,
    voice: &Voice,
    path: &str,
    now_ms: u64,
    explainer: &dyn Explainer,
    declaration_path: &Path,
    // Read while the operation runs, so it can be stopped.
    keyboard: &Keyboard,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(transcriber) = &voice.transcriber else {
        println!("  no transcriber is configured, so audio cannot be understood.");
        println!("  run `adi models check` to see what is missing.");
        return Ok(());
    };

    let audio = read_wav(Path::new(path))?;
    println!(
        "  heard        {} samples at {} Hz",
        audio.samples.len(),
        audio.sample_rate
    );

    // Held before the next line shadows it, so it can be handed on to the
    // loop that waits out the operation. That loop needs the whole handle, not
    // the unwrapped interpreter.
    let mind = interpreter;
    let Some((interpreter, _)) = interpreter else {
        println!("  no interpreter is configured, so this cannot be understood.");
        return Ok(());
    };

    // Through Language::understand rather than the transcriber directly.
    //
    // The library applies a check this skipped when written by hand: a
    // transcript the backend is not confident in becomes a question rather than
    // an intent. Mishearing "hotter" as "shorter" is a physical action on a
    // machine with a heating element, and the transcript is the one place where
    // a wrong word is indistinguishable downstream from a right one.
    let mut language = Language::new(
        // Both reborrowed through their Arc. `Language` wants things
        // implementing the seam traits, which a `&Arc<dyn _>` is not - only
        // the thing inside it is.
        &**transcriber,
        &**interpreter,
        RecordExplainer,
        NoSpeaker,
    );

    // The threshold the configuration states, not the library default. These
    // agree in the shipped examples, which is why the field going unread was
    // invisible: a machine in a noisier place than a kitchen counter needs a
    // higher one, and setting it had no effect at all.
    language.min_transcript_confidence = voice.min_confidence;

    // What the appliance is reading now goes to the interpreter, because a
    // request can be stated entirely against it. "A bit warmer" names no
    // number, and without a reading a model has nothing for warmer to be
    // warmer than and has to ask.
    let readings = session.measurements().clone();

    let recent = voice.recent();
    let context = SelectionContext {
        history: session.history(),
        cycle: None,
        memory: Some(session.memory()),
        rejected: None,
        settings: None,
        measured: Some(&readings),
        // The exchange opened on the command that started the listening
        // rather than on the transcript, which is not yet known here. Harmless
        // while the audio comes from a file; once a microphone opens an
        // exchange of its own, this wants the transcript instead.
        recent: &recent,
            // No style yet. These contexts serve the INTERPRETER, and the
            // style is what the interpreter works out - it cannot be an input
            // to the call that produces it.
            style: None,
};

    // What it heard, before what was made of it. Without this line a misheard
    // word and a model reasoning badly about the right one look identical.
    let (transcript, understood) = language.understand_heard(&audio, &context)?;
    println!(
        "  transcript   \"{}\"  confidence {:.2}",
        transcript.text, transcript.confidence
    );
    // None, because `understand_heard` returns the decision alone. The spoken
    // path gets its acknowledgement when that seam carries one too; until
    // then it falls through to the engine's own line below.
    act_on(
        session,
        understood,
        now_ms,
        voice,
        None,
        declaration_path,
        explainer,
        keyboard,
        mind,
    )
}

fn request(
    session: &mut Session<'_>,
    interpreter: Option<&(std::sync::Arc<dyn Interpreter + Send + Sync>, String)>,
    words: &str,
    now_ms: u64,
    voice: &Voice,
    explainer: &dyn Explainer,
    declaration_path: &Path,
    // Read while the operation runs, so it can be stopped.
    keyboard: &Keyboard,
) -> Result<(), Box<dyn std::error::Error>> {
    // Held before the next line shadows it, so it can be handed on to the
    // loop that waits out the operation. That loop needs the whole handle, not
    // the unwrapped interpreter.
    let mind = interpreter;
    let Some((interpreter, _)) = interpreter else {
        println!("  no interpreter is configured, so this cannot be understood.");
        println!("  run `adi models check` to see what is missing.");
        return Ok(());
    };

    // What the appliance is reading now goes to the interpreter, because a
    // request can be stated entirely against it. "A bit warmer" names no
    // number, and without a reading a model has nothing for warmer to be
    // warmer than and has to ask.
    let readings = session.measurements().clone();

    // Held in a binding: `SelectionContext` borrows it, and a temporary would
    // be dropped at the end of the statement.
    let recent = voice.recent();
    let context = SelectionContext {
        history: session.history(),
        cycle: None,
        memory: Some(session.memory()),
        rejected: None,
        settings: None,
        measured: Some(&readings),
        recent: &recent,
            // No style yet. These contexts serve the INTERPRETER, and the
            // style is what the interpreter works out - it cannot be an input
            // to the call that produces it.
            style: None,
};

    let answer = interpreter.reply(words, &context)?;
    act_on(
        session,
        answer.understood,
        now_ms,
        voice,
        answer.say.as_deref(),
        declaration_path,
        explainer,
        keyboard,
        mind,
    )
}

/// Say what the model offered to say, before anything is sent.
///
/// BEFORE, and that is the trade. The appliance has not accepted yet, so a
/// refusal arrives after the words are out and becomes a correction:
///
///     "Cappuccino, coming right up."
///     "- actually, there is no milk in the hose."
///
/// Which is what a person does, and the correction is immediate rather than
/// twenty seconds later. Waiting for acceptance instead would buy about two
/// hundred milliseconds of accuracy at the cost of the whole point: the
/// request path blocks until the DRINK IS FINISHED, so anything said after it
/// is said a minute late.
///
/// The model is instructed to acknowledge and nothing more - no quantities, no
/// claim that it worked - because those are the parts that could be wrong. The
/// engine owns every sentence that reports a result.
fn acknowledge(say: Option<&str>, voice: &Voice) {
    if let Some(text) = say.map(str::trim).filter(|t| !t.is_empty()) {
        println!("  says         {text}");
        voice.say(text);
    }
}

/// What to do with whatever the model made of an utterance.
///
/// Shared by the typed and the spoken path, so a request that arrived as audio
/// is treated identically to one that was typed. A transcript is text, and
/// nothing past this point needs to know how it got here.
fn act_on(
    session: &mut Session<'_>,
    understood: Understood,
    now_ms: u64,
    voice: &Voice,
    // What the model offered to say while this is carried out, where it
    // offered anything. Spoken on the acting arms only: `Clarify` and `Answer`
    // are already sentences, and saying something before one of those would be
    // talking over the actual reply.
    say: Option<&str>,
    declaration_path: &Path,
    // Says what happened, once it has. `RecordExplainer` where no model backs
    // the seam, which is flat but true.
    explainer: &dyn Explainer,
    // Read while the operation runs, so it can be stopped.
    keyboard: &Keyboard,
    // Passed down so that something said while an operation runs can be
    // UNDERSTOOD rather than only matched against the word "stop". The
    // understanding happens on another thread; see `think`.
    mind: Option<&(std::sync::Arc<dyn Interpreter + Send + Sync>, String)>,

) -> Result<(), Box<dyn std::error::Error>> {
    match understood {
        // ADI-P 9.4.3. Asking is an outcome, not a failure.
        Understood::Clarify { question, because } => {
            println!("  asks         {question}");
            println!("  because      {because}");
            voice.say(&question);
        }
        // Nothing is actuated. The person asked something and is told.
        Understood::Answer { text, drew_on } => {
            println!("  answers      {text}");
            if !drew_on.is_empty() {
                println!("  from         {}", drew_on.join(", "));
            }
            voice.say(&text);
        }
        // A declared tool the person named. ADI-P 8.2.
        //
        // The arm this whole mechanism exists for: before it, "stop making the
        // coffee" and "this is a new cup" had no shape to come back in, so the
        // model answered instead and the machine reported having done
        // something it had not.
        // Several of the machine's own products, in the order given.
        //
        // One at a time through the same path a single one takes, so each gets
        // its envelope check, its confirmation where the declaration flags a
        // hazard, and its own record. A failure stops the rest: somebody who
        // asked for a rinse and then a cappuccino did not ask for the
        // cappuccino into an unrinsed pipe.
        Understood::Actions(actions) => {
            acknowledge(say, voice);
            for (nth, action) in actions.iter().enumerate() {
                if nth > 0 {
                    println!("  then");
                }
                perform_named(
                    session,
                    *action,
                    now_ms,
                    voice,
                    explainer,
                    declaration_path,
                    keyboard,
                    mind,
                )?;
            }
        }

        Understood::Action(action) => {
            acknowledge(say, voice);
            perform_named(
                session,
                action,
                now_ms,
                voice,
                explainer,
                declaration_path,
                keyboard,
                mind,
            )?;
        }
        Understood::Intent(intent) => {
            acknowledge(say, voice);
            println!("  understood   {:?}", intent.kind);
            if let Some(style) = &intent.style {
                println!("  style        {style}");
            }
            for (variable, target) in &intent.targets {
                println!("  target       {} {}", variable.0, describe(target));
                // What the appliance says that outcome is now. The difference
                // between the two is the correction, so printing one without
                // the other shows half of the arithmetic.
                if let Some(now) = session.measurements().get(variable) {
                    println!("  measured     {} {:?}", now.value, now.unit);
                }
            }

            // Started, not waited out. The waiting happens in `attend`,
            // which reads the keyboard while it waits so the drink can be
            // stopped.
            let turn = session.begin(&intent, now_ms)?;
            if attend(session, keyboard, now_ms, voice, mind)? == Attended::Stopped {
                return Ok(());
            }
            if turn.performed.is_empty() {
                println!("  result       nothing needed doing");
            }
            for (record, invocation) in turn.records.iter().zip(&turn.performed) {
                println!("  sent         {}", invocation.tool.as_str());
                // The value, as the cycle path has always printed it. Without
                // it a person watching cannot tell a correction from a message
                // that moved nothing, and those look identical from here.
                for (name, value) in &invocation.numeric_arguments {
                    println!("               {name} = {} {:?}", value.value, value.unit);
                }
                println!("  accepted     by the appliance");

                for (parameter, wanted) in &record.clamped_from {
                    println!(
                        "  at limit     {parameter} could not reach {wanted}; this correction is partial"
                    );
                }

                // The drink is in the cup by now: `request` does not return
                // until the appliance reports the operation complete. So this
                // is the moment a person is waiting to hear about, and the
                // only one where a sentence can be written from what actually
                // happened rather than from what was meant.
                say_what_happened(explainer, record, voice);
            }
        }
    }

    Ok(())
}

/// Say what happened, from the record.
///
/// After the fact, so nothing here can promise something that did not occur.
/// That is the division of labour: the acknowledgement is short and says
/// nothing factual, and every factual sentence is written from a record.
///
/// A failure to explain is reported and nothing more. The machine has already
/// done the right thing; being unable to describe it is a worse outcome than
/// silence only if it stops the work, and it must not.
fn say_what_happened(
    explainer: &dyn Explainer,
    record: &adi_core::record::DecisionRecord,
    voice: &Voice,
) {
    match explainer.explain(record) {
        Ok(text) => {
            println!("  explains     {text}");
            voice.say(&text);
        }
        Err(e) => println!("  explainer    unavailable: {e}"),
    }
}

/// Say why something could not be done.
///
/// The reason comes from the appliance and is already accurate; this only
/// makes it sound like a person said it. Which matters more than it sounds:
/// "a condition that tool requires does not hold" is true and tells nobody
/// what to do about it, where "there is no milk in the hose" does.
fn report_refusal(
    explainer: &dyn Explainer,
    asked: &str,
    error: &(dyn std::error::Error + 'static),
    voice: &Voice,
) {
    let reason = error.to_string();
    let text = explainer
        .explain_refusal(asked, &reason)
        .unwrap_or_else(|_| reason.clone());

    println!("  could not    {text}");
    voice.say(&text);
}

/// Carry out one named action, having asked first where that is warranted.
///
/// Shared by the spoken path and the typed `do` command, so a request that
/// arrived as speech is performed identically to one typed by hand. It also
/// means the mechanism is exercisable with no model and no network, which is
/// the only way to test it on a bench.
fn perform_named(
    session: &mut Session<'_>,
    index: usize,
    now_ms: u64,
    voice: &Voice,
    explainer: &dyn Explainer,
    // For the engine's own actions, which act on files beside it rather than
    // on the appliance.
    declaration_path: &Path,
    // Read while the operation runs, so it can be stopped.
    keyboard: &Keyboard,
    // Passed down so that something said while an operation runs can be
    // UNDERSTOOD rather than only matched against the word "stop". The
    // understanding happens on another thread; see `think`.
    mind: Option<&(std::sync::Arc<dyn Interpreter + Send + Sync>, String)>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Past the appliance's own actions is one of the engine's. Checked first,
    // because `named_actions().get(index)` would return None for one of those
    // and report it as an index this machine does not have - which is the
    // wrong message for an action it does have and performs itself.
    let appliance_actions = session.named_actions().len();
    if index >= appliance_actions {
        return perform_engine_action(index - appliance_actions, declaration_path, voice);
    }

    // Copied out before acting, because the list borrows the session and the
    // next line needs it mutably.
    let Some((name, tool, hazard, delivers)) = session
        .named_actions()
        .get(index)
        .map(|a| {
            (
                a.name.to_string(),
                a.tool.0.clone(),
                a.hazard.to_vec(),
                !a.effects.is_empty(),
            )
        })
    else {
        // Said plainly rather than clamped to the nearest action: the nearest
        // one is a different physical act.
        println!("  cannot       there is no action {index} on this machine");
        println!("  try          `do` on its own lists them");
        return Ok(());
    };

    println!("  named        {name}  ({tool})");

    // ADI-P 4.2: a hazard informs confirmation policy. Nothing used it until
    // now. A word misheard in a noisy room should not start hot water, and the
    // declaration already says which actions deserve the question.
    //
    // But not where the action DELIVERS something. Every drink this machine
    // makes involves hot liquid, so asking before a cappuccino is asking
    // whether someone wants the coffee they just ordered - and a question
    // asked every time is a question that stops being read.
    //
    // What is worth asking about is a routine: it delivers nothing, takes the
    // machine away for minutes, and may consume a tablet. The asymmetry is the
    // argument - a drink nobody wanted costs a cup, a descale nobody wanted
    // costs twenty minutes and a consumable and cannot be interrupted.
    if !hazard.is_empty() && !delivers {
        let question = format!(
            "That involves {}. Shall I go ahead?",
            hazard.join(" and ").replace('_', " ")
        );
        println!("  asks         {question}");
        voice.say(&question);

        if !confirmed(keyboard) {
            println!("  left it      nothing was done");
            return Ok(());
        }
    }

    // Not `?`. A refused action is an outcome a person is owed in words, and
    // propagating it here would end the session over a cup of milk that was
    // not there - the one thing the `do` command exists to be robust against.
    let turn = match session.begin_act(index, now_ms) {
        Ok(turn) => turn,
        Err(error) => {
            if is_fatal(&error) {
                return Err(error.into());
            }
            report_refusal(explainer, &name, &error, voice);
            return Ok(());
        }
    };

    // Nothing below may claim the action was carried out if it was cut
    // short. The lines that follow are written from the record, which says
    // what was INTENDED, so they read as a success whatever happened.
    if attend(session, keyboard, now_ms, voice, mind)? == Attended::Stopped {
        return Ok(());
    }

    for invocation in &turn.performed {
        println!("  did          {}", invocation.tool.as_str());
        println!("  accepted     by the appliance");
    }

    // The action is finished by now - `act` does not return until the
    // appliance reports it complete - so this is the "it is ready" moment,
    // and the first time anything can be said that is grounded in what
    // actually happened.
    for record in &turn.records {
        say_what_happened(explainer, record, voice);
    }

    Ok(())
}

/// Read a yes.
///
/// Anything else is a no, including end of input. A confirmation that defaults
/// to yes when it cannot hear an answer is not a confirmation.
fn confirmed(keyboard: &Keyboard) -> bool {
    // Through the keyboard, not straight from stdin.
    //
    // A second reader on the same stdin is a race: one thread now reads lines
    // continuously into a queue, and a direct `read_line` here would take
    // whichever line the two of them happened to compete for. The symptom is
    // input vanishing - an answer to this question swallowed by the queue, or
    // the next command eaten as the answer.
    match keyboard.next() {
        Some(answer) => matches!(
            answer.trim().to_lowercase().as_str(),
            "y" | "yes" | "go ahead"
        ),
        // Input ended rather than an answer given, which is not consent.
        None => false,
    }
}

/// List what an operator may ask for by name, numbered as the engine numbers
/// them.
/// Actions the ENGINE carries out, not the appliance.
///
/// WHY THESE EXIST AND WHY THEY ARE NOT DECLARED.
///
/// Every tool in a declaration is the appliance's: the engine sends it over
/// the link and the appliance performs it. "Forget what you have learned" is
/// not one of those. What the machine has learned lives in a file on the host,
/// and the coffee machine knows nothing about it - sending it a tool would be
/// asking it to do something it has never heard of.
///
/// But a person asks the MACHINE to forget, not the host, and they ask out
/// loud. So it has to be nameable the way a rinse is nameable, which means it
/// has to be in the list the model is offered.
///
/// So the list the model sees is the appliance's actions followed by these.
/// The index space is shared and the boundary is `named_actions().len()`:
/// below it, the engine sends an invocation; at or above it, the engine acts
/// itself and nothing goes over the link.
///
/// This is a gap in ADI-P rather than a trick. The protocol assumes every
/// operator-nameable action belongs to the appliance, and some things a person
/// asks a machine to do belong to whatever is holding its memory. Recorded in
/// BOB-TODO rather than quietly worked around.
///
/// Deliberately tiny. Anything that moves a physical quantity belongs in the
/// declaration, where the envelope, the interlocks and the hazards apply to
/// it. Nothing here touches the appliance at all.
const ENGINE_ACTIONS: &[(&str, &str)] = &[(
    "forget what you have learned",
    concat!(
        "Delete this machine's learned preferences, so it goes back to ",
        "the manufacturer's own settings. Nothing about the drinks it ",
        "can make changes."
    ),
)];

/// Carry out one of the engine's own actions.
///
/// `which` is an index into `ENGINE_ACTIONS`, already offset by the caller.
fn perform_engine_action(
    which: usize,
    declaration_path: &Path,
    voice: &Voice,
) -> Result<(), Box<dyn std::error::Error>> {
    match which {
        // Forget what has been learned.
        0 => {
            let Some(path) = preferences_beside(declaration_path) else {
                // Nothing to delete is not a failure. A machine asked to
                // forget something it never knew should say so, not error.
                let said = "There is nothing I have learned yet, so there is nothing to forget.";
                println!("  nothing      no learned preferences are stored");
                voice.say(said);
                return Ok(());
            };

            std::fs::remove_file(&path)?;

            println!("  forgot       {}", path.display());
            let said = concat!(
                "I have forgotten what I had learned. I am back to the ",
                "manufacturer's settings from your next drink."
            );
            voice.say(said);
            Ok(())
        }
        // Unreachable while the list has one entry, and a wrong index here
        // would act on something nobody asked for.
        other => Err(format!("the engine has no action {other}").into()),
    }
}

fn list_named(session: &Session<'_>) {
    let actions = session.named_actions();
    if actions.is_empty() {
        println!("  this machine declares nothing an operator can name.");
        println!("  every tool it has takes a value, so the engine chooses them.");
        return;
    }

    println!("  actions you can ask for by name:");
    for action in &actions {
        let hazard = if action.hazard.is_empty() {
            String::new()
        } else {
            "   (asks first)".to_string()
        };
        println!("    do {}   {}{}", action.index, action.name, hazard);
        if let Some(description) = action.description {
            println!("            {}", description.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }

    // The engine's own, after the appliance's and numbered on from them, which
    // is the order `perform_named` resolves an index against. `do` has to be
    // the whole truth about what can be asked for, or a person reads this list
    // and concludes something is impossible when it is not.
    for (offset, (name, description)) in ENGINE_ACTIONS.iter().enumerate() {
        println!("    do {}   {}", actions.len() + offset, name);
        println!(
            "            {}",
            description.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
}

fn describe(target: &adi_protocol::intent::TargetSpec) -> String {
    use adi_protocol::intent::{Reference, TargetSpec};
    match target {
        TargetSpec::Absolute(q) => format!("= {} {:?}", q.value, q.unit),
        TargetSpec::Relative(r) => {
            let of = match &r.of {
                Reference::Last => "last".to_string(),
                Reference::Now => "now".to_string(),
                Reference::Cycle(id) => id.clone(),
            };
            match (r.scale, &r.offset) {
                (Some(s), _) => format!("= {s} x {of}"),
                (None, Some(o)) => format!("= {of} {:+} {:?}", o.value, o.unit),
                (None, None) => format!("= same as {of}"),
            }
        }
    }
}

/// Which level to run at.
///
/// ADI-P 3.3 makes D2 the default. Stated explicitly rather than inferred from
/// what happens to be configured, so a developer who asked for a level and did
/// not get it is told rather than left to read the banner.
fn parse_level(requested: Option<&str>) -> Result<AutonomyLevel, Box<dyn std::error::Error>> {
    Ok(match requested.map(str::to_lowercase).as_deref() {
        None | Some("d2") => AutonomyLevel::D2,
        Some("d1") => AutonomyLevel::D1,
        Some("d0") => AutonomyLevel::D0,
        Some(other) => return Err(format!("unknown level '{other}'; expected d0, d1 or d2").into()),
    })
}

/// Start the chooser a model-driven level needs.
///
/// D0 needs none, so this returns nothing for it rather than starting a
/// program that would never be asked anything.
/// Whatever will say what happened.
///
/// `RecordExplainer` unless a configuration names a program for the explain
/// seam. That fallback is not a placeholder: the domain rules already write an
/// explanation when they diagnose, so a machine with no model still says
/// something true. What a model adds is a sentence a person wants to hear
/// rather than one they have to parse.
///
/// Infallible on purpose. Every other seam refuses loudly when it is
/// misconfigured, because without it the machine cannot act. This one only
/// changes how the machine TALKS, so a broken explainer must not stop a drink
/// being made - it falls back and says so.
fn load_explainer(
    config_path: Option<&Path>,
    shared: Option<&Arc<ProcessBackend>>,
) -> Box<dyn Explainer> {
    let Some(path) = config_path else {
        return Box::new(RecordExplainer);
    };
    let Ok(config) = ModelsConfig::load(path) else {
        return Box::new(RecordExplainer);
    };

    let seam = config.seam(Seam::Explainer);
    if seam.backend != Backend::Process {
        return Box::new(RecordExplainer);
    }

    // Over the program already running where there is one, so the four seams
    // are one process rather than four.
    if let Some(backend) = shared {
        return Box::new(ProcessExplainer::over(Arc::clone(backend)));
    }

    let Some(command) = seam.command.clone() else {
        return Box::new(RecordExplainer);
    };
    match ProcessExplainer::start(&command, &seam.args) {
        Ok(explainer) => Box::new(explainer),
        Err(e) => {
            println!("  explainer    unavailable: {e}; falling back to the record");
            Box::new(RecordExplainer)
        }
    }
}

/// Whatever works out what the machine has learned, if anything does.
///
/// Served by the explain seam's program rather than a seam of its own: both
/// jobs look at records after the fact, and the message shape tells them
/// apart. Only over a program already running - there is no reason to start a
/// second copy for a job that happens occasionally.
///
/// None where no model serves that seam, and that is a complete answer. A
/// machine with no model learns nothing, which is better than a machine that
/// learns by a rule somebody guessed.
fn load_learner(
    config_path: Option<&Path>,
    shared: Option<&Arc<ProcessBackend>>,
) -> Option<ProcessLearner> {
    let backend = shared?;
    let config = ModelsConfig::load(config_path?).ok()?;
    (config.seam(Seam::Explainer).backend == Backend::Process)
        .then(|| ProcessLearner::over(Arc::clone(backend)))
}

/// Ask what has been learned, and write it down if anything was.
///
/// WHEN THIS RUNS. After a request that corrected something. A drink nobody
/// adjusted teaches nothing, so there is nothing to ask about, and asking
/// anyway would spend a model call on every coffee.
///
/// WHAT IT CANNOT DO. Apply the result to the session it is running in. The
/// session borrows the domain immutably for its whole life, so a layer cannot
/// be added underneath it. A learned preference therefore takes effect on the
/// NEXT run, and this says so rather than implying otherwise.
///
/// That is a limitation of the plumbing and not of the idea. It is also
/// arguably the better behaviour: a preference that changed the drink being
/// poured would be learning applied before anybody could object to it.
fn remember(
    learner: Option<&ProcessLearner>,
    session: &Session<'_>,
    domain: &dyn adi_core::domain::Domain,
    declaration_path: &Path,
    voice: &Voice,
) {
    let Some(learner) = learner else {
        return;
    };

    // Only where this person has stated the same preference more than once.
    //
    // WHY THE ENGINE DECIDES THIS AND NOT THE MODEL. The learner used to be
    // asked after any turn that corrected anything, and a `Produce` request
    // corrects whatever the recipe says - so the first drink of a machine's
    // life produced a "learned preference" for the recipe's own brew
    // temperature, written to disk, having been told nothing by anybody. It
    // narrowed again on each later drink, because each later drink said the
    // same thing again, and once the band had crept past a recipe's lower
    // bound the machine could no longer make that drink at all.
    //
    // The prompt did say to look for a preference asked for more than once.
    // Prompts are not where this belongs: whether a person stated a figure is
    // a fact the engine holds, and the record now says which. So the gate is
    // deterministic and the model is only consulted once there is something
    // real to generalise from.
    // Counted off the record's own wording, which `Asked` writes in one place:
    // "asked for ..." where a person named the figure, "the recipe puts ..."
    // where the style did. The record carries no target variable of its own,
    // so this counts stated moments rather than stated moments per variable -
    // enough to decide whether there is anything to generalise from, and the
    // learner is still the one that has to find a pattern among them.
    const ENOUGH: usize = 2;
    let stated = session
        .history()
        .iter()
        .filter(|record| record.explanation.starts_with("asked for"))
        .count();
    if stated < ENOUGH {
        return;
    }

    // The ranges a proposal may narrow, sent so the model works inside them
    // rather than being refused afterwards for not knowing them.
    let current: Vec<(String, f64, f64, String)> = domain
        .knowledge()
        .constrained_variables()
        .into_iter()
        .filter_map(|name| {
            let bounds = domain.knowledge().constraint(name)?;
            Some((
                name.to_string(),
                bounds.value.min,
                bounds.value.max,
                format!("{:?}", bounds.value.unit).to_lowercase(),
            ))
        })
        .collect();

    if current.is_empty() {
        return;
    }

    let learned = match learner.consider(session.history(), &current) {
        Ok(learned) => learned,
        // Reported and dropped. Learning is a convenience; a machine that
        // stops working because it could not decide what it had learned is
        // worse than one that learns nothing.
        Err(e) => {
            eprintln!("  learning     unavailable: {e}");
            return;
        }
    };

    if learned.is_empty() {
        return;
    }

    // Every proposal checked against what is in force. The layering would
    // refuse a widening one at load time anyway, but a proposal that could
    // never be applied should not be written to a file, and the reason is
    // more useful here than next time the machine starts.
    let mut keeping = Vec::new();
    for proposal in &learned.constraints {
        match current
            .iter()
            .find(|(name, _, _, _)| name == &proposal.variable)
        {
            Some((_, min, max, unit)) if adi_models::learner::narrows(proposal, *min, *max, unit) => {
                keeping.push(proposal.clone());
            }
            Some((name, min, max, unit)) => {
                println!(
                    "  learning     ignored {name} {} to {} {}: outside the {} to {} {} in force",
                    proposal.min, proposal.max, proposal.unit, min, max, unit
                );
            }
            None => println!(
                "  learning     ignored {}: nothing constrains it",
                proposal.variable
            ),
        }
    }

    if keeping.is_empty() {
        return;
    }

    match write_preferences(declaration_path, &keeping, &learned.because) {
        Ok(path) => {
            println!("  learned      {}", path.display());
            for proposal in &keeping {
                println!(
                    "               {} {} to {} {}",
                    proposal.variable, proposal.min, proposal.max, proposal.unit
                );
            }
            println!("  because      {}", learned.because);

            // Said out loud, because a machine that changes how it works
            // without mentioning it is the thing this whole project is
            // arguing against. The person can delete the file.
            let said = format!(
                "I have noted that {}. It will apply from your next drink, and you can tell me to forget it.",
                learned.because.trim_end_matches('.')
            );
            voice.say(&said);
        }
        Err(e) => eprintln!("  learning     could not be written: {e}"),
    }
}

/// Write the user layer, replacing whatever was there.
///
/// In the CLI and not the engine. The library does no file I/O anywhere else
/// and should not start here: an engine embedded in an appliance may have no
/// filesystem at all, so the host decides where knowledge is kept and the
/// engine only decides what it is.
fn write_preferences(
    declaration_path: &Path,
    constraints: &[adi_models::LearnedConstraint],
    because: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let stem = declaration_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("the declaration has no file name to derive from")?;
    let stem = stem.strip_suffix(".knowledge").unwrap_or(stem);
    let path = declaration_path.with_file_name(format!("{stem}.user.yaml"));

    let mut out = String::new();
    out.push_str("# What this machine has learned about the person using it.\n");
    out.push_str("#\n");
    out.push_str("# WRITTEN BY THE MACHINE. Delete this file to forget everything in it.\n");
    out.push_str("#\n");
    // One push per line. A backslash continuation inside a Rust string
    // keeps the next line's indentation, which put source-code whitespace
    // into these YAML comments the first time this ran.
    out.push_str("# This is ADI-P 6.1's topmost knowledge layer, so everything
");
    out.push_str("# here can only NARROW what the manufacturer declared. A
");
    out.push_str("# range that widened one is refused when this file is read,
");
    out.push_str("# so learning can tune this machine and cannot redefine what
");
    out.push_str("# it does.
");
    out.push_str("#\n");
    out.push_str("# Why:\n");
    for line in because.lines() {
        out.push_str("#   ");
        out.push_str(line.trim());
        out.push('\n');
    }
    out.push('\n');
    out.push_str("namespace: \"x.bob.user\"\n");
    out.push_str("extension_version: \"1.0\"\n");
    out.push_str("extends:\n");
    out.push_str("  domain_profile: \"espresso/v1\"\n");
    out.push_str("  base_min_version: \"1.0\"\n\n");
    out.push_str("constraints:\n");
    for c in constraints {
        out.push_str(&format!(
            "  {}: {{ min: {}, max: {}, unit: {} }}\n",
            c.variable, c.min, c.max, c.unit
        ));
    }

    // The reason, where the model can read it back and answer "why" with it.
    out.push_str("\nnotes:\n");
    out.push_str("  - id: \"x.bob.user.note.learned\"\n");
    out.push_str("    text: >\n");
    for line in because.lines() {
        out.push_str("      ");
        out.push_str(line.trim());
        out.push('\n');
    }
    out.push_str("
      The manufacturer's own limits are unchanged and still
");
    out.push_str("      apply; this only sits inside them, and it can be
");
    out.push_str("      forgotten on request.
");

    std::fs::write(&path, out)?;
    Ok(path)
}

fn load_chooser(
    config_path: Option<&Path>,
    level: AutonomyLevel,
    shared: Option<&Arc<ProcessBackend>>,
) -> Result<Option<(ProcessChooser, String)>, Box<dyn std::error::Error>> {
    if level == AutonomyLevel::D0 {
        return Ok(None);
    }

    let Some(path) = config_path else {
        return Ok(None);
    };

    let config = ModelsConfig::load(path)?;
    let seam = config.seam(Seam::Interpreter);

    // The chooser runs the same program as the interpreter, and now the same
    // process: one model answers both questions, which is what an appliance
    // with one model resident actually does. `shared` is that program, where
    // the caller started one.
    if seam.backend != Backend::Process {
        return Ok(None);
    }

    let command = seam.command.clone().ok_or("the process backend names no command")?;
    let tag = if level == AutonomyLevel::D2 { "d2" } else { "d1" };
    let label = format!("process ({command}) at {tag}");

    // A chooser that will not start is reported as absent rather than
    // propagated, so the caller's explanation runs: a level that needs a
    // chooser and has none is told what to configure or to run at D0. The
    // bare backend error said only that a program was not found, which does
    // not say what it was for or what to do about it.
    if let Some(backend) = shared {
        return Ok(Some((ProcessChooser::over(Arc::clone(backend), tag), label)));
    }

    match ProcessChooser::start(&command, &seam.args, tag) {
        Ok(chooser) => Ok(Some((chooser, label))),
        Err(e) => {
            println!("  chooser      unavailable: {e}");
            Ok(None)
        }
    }
}

/// An interpreter and a description of where it came from, for the banner.
/// A loaded interpreter, shareable across threads.
///
/// `Arc` and `Send + Sync` rather than `Box`, because understanding an
/// utterance has to happen OFF the loop that carries the keepalive. The legs
/// measure 0.75 s to transcribe, one to three to interpret and three and a
/// half to speak - five to seven all told, against Bob's `control_timeout_ms`
/// of six. An appliance that hears nothing for that long declares its
/// controller dead and abandons the drink, so a spoken question mid-brew would
/// have cost the coffee it was asking about.
///
/// The bound is on the handle here and NOT on the `Interpreter` trait, which
/// stays as it is: an engine on a single-threaded appliance has no use for it,
/// and the trait is public.
type Loaded = Option<(std::sync::Arc<dyn Interpreter + Send + Sync>, String)>;

/// The interpreter seam, where it is configured.
///
/// A seam that cannot be made ready is reported and left unserved, for the
/// same reason the speech seams are: the session still has `cycle` and
/// `status`, which need no model, and ending it here would take those away
/// too. The caller already prints that spoken requests are unavailable.
/// Start the program that will serve the interpreter seam, and the chooser
/// with it.
///
/// `None` where the configuration names no process backend for that seam, in
/// which case each seam falls back to starting its own, which is what a
/// scripted or absent backend wants. A failure here is not reported: the
/// seams report it themselves, in their own words, when they try to start.
fn shared_model_process(config_path: Option<&Path>) -> Option<Arc<ProcessBackend>> {
    let config = ModelsConfig::load(config_path?).ok()?;
    let seam = config.seam(Seam::Interpreter);

    if seam.backend != Backend::Process {
        return None;
    }

    let command = seam.command.clone()?;
    ProcessBackend::start(&command, &seam.args).ok().map(Arc::new)
}

fn load_interpreter(
    config_path: Option<&Path>,
    domain: &dyn adi_core::domain::Domain,
    declaration: &adi_protocol::declaration::Declaration,
    skill: Option<&adi_protocol::skill::Skill>,
    shared: Option<&Arc<ProcessBackend>>,
) -> Result<Loaded, Box<dyn std::error::Error>> {
    match start_interpreter(config_path, domain, declaration, skill, shared) {
        Ok(Some((interpreter, label))) => {
            println!("  interpreter  {label}");
            Ok(Some((interpreter, label)))
        }
        Ok(None) => {
            println!("  interpreter  none, so spoken requests are unavailable");
            Ok(None)
        }
        Err(e) => {
            println!("  interpreter  unavailable: {e}");
            println!("               spoken requests are unavailable; the rest of the session is not");
            Ok(None)
        }
    }
}

/// Build whichever interpreter the configuration names.
///
/// Shared with `adi models bench`, which needs the same backend a session
/// uses. Measuring a model through a second construction of it would measure
/// the second construction.
pub(crate) fn start_interpreter(
    config_path: Option<&Path>,
    domain: &dyn adi_core::domain::Domain,
    declaration: &adi_protocol::declaration::Declaration,
    // Needed to work out which styles this machine can actually produce: a
    // candidate for an outcome may come from a skill response rather than from
    // a tool's declared effects, and a planner without the skill would judge
    // some styles unproducible that are not.
    skill: Option<&adi_protocol::skill::Skill>,
    shared: Option<&Arc<ProcessBackend>>,
) -> Result<Loaded, Box<dyn std::error::Error>> {
    let Some(path) = config_path else {
        return Ok(None);
    };

    let config = ModelsConfig::load(path)?;
    let seam = config.seam(Seam::Interpreter);

    match seam.backend {
        Backend::None => Ok(None),

        Backend::Scripted => {
            let model = config
                .model_path(Seam::Interpreter)
                .ok_or("the scripted backend names no answers file")?;
            let label = format!("scripted ({})", model.display());
            Ok(Some((std::sync::Arc::new(ScriptedInterpreter::load(&model)?), label)))
        }

        Backend::Process => {
            let command = seam.command.clone().ok_or("the process backend names no command")?;

            // Read before the program is started, so a budget that makes no
            // sense is refused rather than discovered after a child process is
            // running.
            let budget = context_budget(&seam)?;

            // Reported, because the whole fault this fixes was a number that
            // looked set and was not. A budget visible in the banner is one a
            // person can see is wrong.
            let label = format!(
                "process ({command}), {} tokens, {} decisions",
                budget.max_tokens, budget.history
            );

            let knowledge = domain.knowledge();
            let planner = adi_core::Planner::new(declaration, skill, domain);
            let surroundings = Surroundings {
                // With what each style is, not only its identifier. A
                // request names a style the way a person would.
                //
                // AND ONLY STYLES THIS MACHINE CAN PRODUCE. A base domain
                // profile describes a CLASS of machine: the espresso profile
                // writes its styles in `ratio` and `shot_time`, which is how a
                // machine with a paddle and a scale is dialled in. A
                // superautomatic can do neither - you give it a volume and it
                // decides the rest - so offering those styles was a trap.
                // Asked for a lungo, a model picked `adi.espresso.style.lungo`,
                // a reasonable answer from the list it was handed, and the only
                // target Bob could serve was the brew temperature. It set the
                // temperature and announced a lungo that was never brewed,
                // while the vendor's own `x.bob.style.lungo` sat in the same
                // list unused.
                //
                // THE TEST: every outcome the recipe specifies must be one
                // this machine can move, preferences aside.
                //
                // A recipe is a specification, not a wish list. The base lungo
                // asks for a ratio of 3 to 4, a shot of 28 to 40 seconds and a
                // temperature; Bob can serve the temperature and neither of
                // the others, so what it would produce is not a lungo - it is
                // a temperature change with a lungo's name on it. Its own
                // `x.bob.style.lungo` asks for a yield mass, which is exactly
                // what this machine delivers.
                //
                // Preferences are excluded from the test because a person can
                // decline one: a recipe that mentions syrup is still that
                // recipe on a machine with no syrup pump.
                //
                // Requiring only SOME target to be servable was the first
                // attempt and let the base styles straight back through -
                // `brew_temperature` is servable and, being a process variable
                // rather than a preference, counted. One of three targets
                // honoured is not the drink.
                //
                // Not an exclusion of base styles as such: a base style this
                // machine can serve in full stays on offer, which is the whole
                // point of having one.
                //
                // AND WHERE NONE QUALIFY, EVERYTHING IS OFFERED. The filter is
                // there to stop a machine recommending a recipe written in a
                // mechanism it does not have. On a machine where that
                // describes every recipe it emptied the list instead, and the
                // question became "I can make: . Which?" - which is worse than
                // an imperfect list, because it asks somebody to choose from
                // nothing.
                //
                // The shipped example appliance is exactly that machine: it
                // serves `brew_temperature` and none of the other variables
                // the base profile's recipes are written in. Bob is unaffected
                // either way, since its own eight styles are producible in
                // full and the base ones still drop out.
                styles: {
                    let described = knowledge.styles_described();

                    let producible: Vec<_> = described
                        .iter()
                        .copied()
                        .filter(|(name, _)| {
                            let Ok(resolved) = knowledge.resolve(name, None) else {
                                return false;
                            };
                            // The STYLE's own bands, not the fully resolved
                            // set. `targets` starts from the machine's
                            // constraints and so names every constrained
                            // variable - ratio and shot time among them -
                            // which failed every style on Bob including its
                            // own. `style_targets` is what the recipe actually
                            // specifies.
                            resolved.style_targets.iter().all(|(variable, _)| {
                                knowledge.is_preference(variable)
                                    || !planner
                                        .candidates(&adi_protocol::ids::VariableId(
                                            variable.clone(),
                                        ))
                                        .is_empty()
                            })
                        })
                        .collect();

                    let offered = if producible.is_empty() {
                        described
                    } else {
                        producible
                    };

                    offered
                        .into_iter()
                        .map(|(name, description)| {
                            // The recipe's own bands, where they can be
                            // resolved. `style_targets` again, not `targets`:
                            // the latter folds in the profile's quality
                            // limits, which say where a variable may NOT go
                            // rather than what this drink is.
                            let targets = knowledge
                                .resolve(name, None)
                                .map(|resolved| {
                                    resolved
                                        .style_targets
                                        .iter()
                                        .map(|(variable, target)| {
                                            let band = &target.value;
                                            // THE AIM FIRST, the band after.
                                            //
                                            // Showing the band alone made
                                            // things worse, measurably: a
                                            // model that could see 90 to 130
                                            // helpfully supplied 95, and a
                                            // lungo came out at 95
                                            // millilitres instead of 117.
                                            // There was nothing wrong with
                                            // its answer - 95 is inside the
                                            // recipe - except that the engine
                                            // was aiming at the middle and
                                            // nothing had ever told the model
                                            // that. Two rules for one number.
                                            //
                                            // With the aim stated they agree,
                                            // and a figure the model does
                                            // send is the same one the engine
                                            // would have chosen.
                                            (
                                                variable.clone(),
                                                match band.aim {
                                                    Some(aim) => format!(
                                                        "{} {} (may be {} to {})",
                                                        aim,
                                                        band.unit.as_str(),
                                                        band.min,
                                                        band.max
                                                    ),
                                                    None => format!(
                                                        "{} to {} {}",
                                                        band.min,
                                                        band.max,
                                                        band.unit.as_str()
                                                    ),
                                                },
                                            )
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();

                            adi_language::context::StyleInfo {
                                name: name.to_string(),
                                description: description.map(str::to_string),
                                targets,
                            }
                        })
                        .collect()
                },
                materials: knowledge
                    .material_ids()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                // Each with its unit and meaning, so the model is not asked
                // to infer what a variable is from its name alone.
                variables: domain
                    .variables()
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
                // The manufacturer's own material, so the machine can answer
                // a question rather than only act on a request.
                notes: knowledge.notes().cloned().collect(),
                device: Some(format!(
                    "{} {}, firmware {}",
                    declaration.device.vendor,
                    declaration.device.model,
                    declaration.device.firmware_version
                )),
                // The things this machine does that are not quantities.
                //
                // Derived here, from the declaration, so that the list the
                // model is shown and the list `Session::act` resolves against
                // are the same list in the same order. Two derivations would
                // be two orders eventually, and an index resolved against the
                // wrong one performs the wrong physical act.
                // The appliance's, then the engine's own. Same order the
                // index is resolved against in `perform_named`, which is the
                // only thing keeping an index pointing at what it names.
                actions: adi_core::named_actions(declaration, domain)
                    .into_iter()
                    .map(|a| adi_language::context::ActionLine {
                        name: a.name.to_string(),
                        description: a.description.map(str::to_string),
                    })
                    .chain(
                        ENGINE_ACTIONS
                            .iter()
                            .map(|(name, description)| adi_language::context::ActionLine {
                                name: name.to_string(),
                                description: Some(description.to_string()),
                            }),
                    )
                    .collect(),
            };

            // Over the program already running where there is one, so that
            // this seam and the chooser are the same process.
            let interpreter = match shared {
                Some(backend) => ProcessInterpreter::over(
                    Arc::clone(backend),
                    surroundings,
                    budget,
                ),
                None => ProcessInterpreter::start(
                    &command,
                    &seam.args,
                    surroundings,
                    budget,
                )?,
            };
            Ok(Some((std::sync::Arc::new(interpreter), label)))
        }

        other => Err(format!(
            "the '{}' backend is not implemented; use 'process'",
            other.as_str()
        )
        .into()),
    }
}

fn read(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()).into())
}

fn default_appliance() -> PathBuf {
    resolve_executable(Path::new("c/build/example"))
}

/// Accept a path written without a platform's executable suffix.
///
/// A path given in documentation or a script is written once and run on
/// several platforms. `c/build/example` is the binary on Linux and macOS and
/// `example.exe` on Windows, and refusing one because it was written as the
/// other is a failure with no useful cause to report.
fn resolve_executable(given: &Path) -> PathBuf {
    if given.exists() {
        return given.to_path_buf();
    }

    let with_exe = given.with_extension(std::env::consts::EXE_EXTENSION);
    if with_exe.exists() {
        return with_exe;
    }

    given.to_path_buf()
}

#[cfg(test)]
mod voice_config {
    //! Reading the speech seams' configuration.
    //!
    //! Both numbers below were present in the shipped example configurations,
    //! documented in the type that holds them, and read by nothing. The rate
    //! was hardcoded at the call site and the threshold was taken from the
    //! library default, which happens to equal the value in the examples, so
    //! the field appeared to work.

    use super::{
        context_budget, speech_rate, transcript_threshold, ContextBudget, DEFAULT_SPEAKER_RATE,
        DEFAULT_TRANSCRIBER_RATE,
    };
    use adi_models::{Backend, SeamConfig};

    fn seam(sample_rate: Option<u32>, min_confidence: Option<f64>) -> SeamConfig {
        SeamConfig {
            backend: Backend::Process,
            sample_rate,
            min_confidence,
            ..SeamConfig::none()
        }
    }

    fn budgeted(context_tokens: Option<usize>) -> SeamConfig {
        SeamConfig {
            backend: Backend::Process,
            context_tokens,
            ..SeamConfig::none()
        }
    }

    /// The third field in this family to be written down and read by nothing.
    ///
    /// Its effect was the worst of the three: a wrong sample rate plays audio
    /// at the wrong pitch, which anyone notices, while a budget stuck at the
    /// default silently deleted part of every prompt.
    #[test]
    fn a_configured_budget_is_what_is_used() {
        assert_eq!(
            context_budget(&budgeted(Some(12_000))).map(|b| b.max_tokens),
            Ok(12_000)
        );
    }

    /// A configuration that says nothing behaves as it always did.
    #[test]
    fn an_unset_budget_is_the_default() {
        assert_eq!(
            context_budget(&budgeted(None)).map(|b| b.max_tokens),
            Ok(ContextBudget::default().max_tokens)
        );
    }

    /// Refused rather than read as "no limit", because zero reads either way
    /// and the two are opposites.
    #[test]
    fn a_budget_of_zero_is_refused() {
        assert!(context_budget(&budgeted(Some(0))).is_err());
    }

    /// The other two fields keep their defaults, so configuring the size of
    /// the prompt does not quietly change how much history is in it.
    #[test]
    fn setting_the_budget_leaves_the_other_limits_alone() {
        let configured = context_budget(&budgeted(Some(9_000))).expect("valid");
        let default = ContextBudget::default();

        assert_eq!(configured.remembered_cycles, default.remembered_cycles);
        assert_eq!(configured.history, default.history);
    }

    #[test]
    fn a_configured_rate_is_what_is_used() {
        assert_eq!(speech_rate(&seam(Some(24_000), None), DEFAULT_SPEAKER_RATE), Ok(24_000));
        assert_eq!(speech_rate(&seam(Some(8_000), None), DEFAULT_TRANSCRIBER_RATE), Ok(8_000));
    }

    /// Piper publishes voices at 16 and at 22.05 kHz. A tool that reads only
    /// its own default can run half of them.
    #[test]
    fn a_rate_other_than_the_default_is_not_overridden() {
        let configured = speech_rate(&seam(Some(16_000), None), DEFAULT_SPEAKER_RATE);
        assert_eq!(configured, Ok(16_000), "the speaker default must not win");
    }

    #[test]
    fn no_rate_means_the_default_for_that_seam() {
        assert_eq!(speech_rate(&seam(None, None), DEFAULT_TRANSCRIBER_RATE), Ok(16_000));
        assert_eq!(speech_rate(&seam(None, None), DEFAULT_SPEAKER_RATE), Ok(22_050));
    }

    /// A duration is a sample count divided by a rate.
    #[test]
    fn a_rate_of_zero_is_refused() {
        assert!(speech_rate(&seam(Some(0), None), DEFAULT_SPEAKER_RATE).is_err());
    }

    #[test]
    fn a_configured_threshold_is_what_is_used() {
        assert_eq!(transcript_threshold(&seam(None, Some(0.9))), Ok(0.9));
        assert_eq!(transcript_threshold(&seam(None, Some(0.0))), Ok(0.0));
        assert_eq!(transcript_threshold(&seam(None, Some(1.0))), Ok(1.0));
    }

    #[test]
    fn no_threshold_means_the_library_default() {
        assert_eq!(
            transcript_threshold(&seam(None, None)),
            Ok(adi_language::DEFAULT_MIN_TRANSCRIPT_CONFIDENCE)
        );
    }

    /// A threshold outside 0 to 1 can never be met or can never be missed, and
    /// either makes the check decorative. Refused rather than clamped: a
    /// clamped threshold is one nobody chose and nobody can see.
    #[test]
    fn a_threshold_outside_zero_to_one_is_refused() {
        assert!(transcript_threshold(&seam(None, Some(1.5))).is_err());
        assert!(transcript_threshold(&seam(None, Some(-0.1))).is_err());
        assert!(transcript_threshold(&seam(None, Some(f64::NAN))).is_err());
        assert!(transcript_threshold(&seam(None, Some(f64::INFINITY))).is_err());
    }
}
