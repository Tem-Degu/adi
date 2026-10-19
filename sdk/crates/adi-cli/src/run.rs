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

use adi_core::{Cycle, SubjectiveFeedback};
use adi_decision::{
    AutonomyLevel, Interpreter, ModelActionSelector, ModelToolSelector, SelectionContext,
    Understood,
};
use adi_language::context::VariableInfo;
use adi_language::{Explainer, Language, NoSpeaker, RecordExplainer, Speaker, Transcriber};
use adi_models::{
    ProcessSpeaker, ProcessTranscriber, SeamConfig, Surroundings,
    Backend, ModelsConfig, ProcessChooser, ProcessInterpreter, ScriptedInterpreter, Seam,
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
    // naming a level that did not run is the one thing 3.3 forbids.
    let chooser = load_chooser(config_path.as_deref(), requested)?;

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

    let interpreter = load_interpreter(config_path.as_deref(), domain.as_ref(), &declaration.device)?;
    let voice = Voice::load(config_path.as_deref())?;

    println!();
    help();

    let stdin = std::io::stdin();
    let mut now_ms: u64 = 0;

    loop {
        print!("> ");
        std::io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        now_ms += 60_000;

        // A failed command is reported and the session continues. Propagating
        // it ended the session on the first refusal, which is wrong twice over:
        // a refusal is frequently the correct outcome, and a person who has
        // just been refused is the one most likely to want to ask something
        // else. Only a broken link ends a session, and that is handled below.
        let outcome = match line {
            "" => continue,
            "quit" | "exit" => break,
            "help" => {
                help();
                Ok(())
            }
            "status" => status(&mut session, now_ms),
            "cycle" => cycle(&mut session, &declaration.domain_profile, now_ms, &voice),
            // A recorded utterance, since capturing from a microphone needs a
            // device and a platform library this tool does not carry.
            other if other.starts_with("listen ") => listen(
                &mut session,
                interpreter.as_ref(),
                &voice,
                other.trim_start_matches("listen ").trim(),
                now_ms,
            ),
            _ => request(&mut session, interpreter.as_ref(), line, now_ms, &voice),
        };

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
    speaker: Option<Box<dyn Speaker>>,
    transcriber: Option<Box<dyn Transcriber>>,
    /// Below this a transcript is treated as not heard, and the machine asks
    /// instead of acting. Carried here because it is read from the
    /// configuration and applied where a transcript is produced.
    min_confidence: f64,
}

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
                            Some(Box::new(speaker) as Box<dyn Speaker>)
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
                            Some(Box::new(transcriber) as Box<dyn Transcriber>)
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
            min_confidence,
        })
    }

    /// Say something, where a speaker is configured.
    ///
    /// The audio is written beside the working directory rather than played.
    /// Playback needs an output device and a platform library, which is a
    /// packaging concern; what matters to the protocol is that the text reached
    /// the seam and came back as samples.
    fn say(&self, text: &str) {
        let Some(speaker) = &self.speaker else {
            return;
        };

        match speaker.speak(text) {
            Ok(audio) => {
                let path = std::path::Path::new("spoken.wav");
                match write_wav(&path.to_path_buf(), &audio) {
                    Ok(()) => println!(
                        "  spoken       {} samples at {} Hz, written to {}",
                        audio.samples.len(),
                        audio.sample_rate,
                        path.display()
                    ),
                    Err(e) => println!("  speaker      could not write the audio: {e}"),
                }
            }
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
    } else if text.contains("appliance refused") {
        println!("  refused       the appliance declined it: {text}");
    } else if text.contains("both move") || text.contains("one iteration") {
        println!("  refused       the plan changed one thing two ways, which would make");
        println!("                the result impossible to attribute. Nothing ran.");
    } else {
        println!("  failed        {text}");
    }
}

/// Whether the session can usefully continue.
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
fn cycle(
    session: &mut Session<'_>,
    profile: &str,
    now_ms: u64,
    voice: &Voice,
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

        // The explainer composes a sentence from the record. It needs no model,
        // which is why a machine with no voice still has something to say.
        match RecordExplainer.explain(record) {
            Ok(text) => {
                println!("  explains     {text}");
                voice.say(&text);
            }
            Err(e) => println!("  explainer    unavailable: {e}"),
        }
    }

    Ok(())
}

fn status(session: &mut Session<'_>, now_ms: u64) -> Result<(), Box<dyn std::error::Error>> {
    session.tick(now_ms)?;
    println!("  link         alive");
    println!("  cycles held  {}", session.memory().len());
    println!("  decisions    {}", session.history().len());
    Ok(())
}

/// The request path: a person asked for something in words.
fn listen(
    session: &mut Session<'_>,
    interpreter: Option<&(Box<dyn Interpreter>, String)>,
    voice: &Voice,
    path: &str,
    now_ms: u64,
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
        transcriber,
        interpreter,
        RecordExplainer,
        NoSpeaker,
    );

    // The threshold the configuration states, not the library default. These
    // agree in the shipped examples, which is why the field going unread was
    // invisible: a machine in a noisier place than a kitchen counter needs a
    // higher one, and setting it had no effect at all.
    language.min_transcript_confidence = voice.min_confidence;

    let context = SelectionContext {
        history: session.history(),
        cycle: None,
        memory: Some(session.memory()),
        rejected: None,
    };

    let understood = language.understand(&audio, &context)?;
    act_on(session, understood, now_ms, voice)
}

fn request(
    session: &mut Session<'_>,
    interpreter: Option<&(Box<dyn Interpreter>, String)>,
    words: &str,
    now_ms: u64,
    voice: &Voice,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some((interpreter, _)) = interpreter else {
        println!("  no interpreter is configured, so this cannot be understood.");
        println!("  run `adi models check` to see what is missing.");
        return Ok(());
    };

    let context = SelectionContext {
        history: session.history(),
        cycle: None,
        memory: Some(session.memory()),
        rejected: None,
    };

    let understood = interpreter.interpret(words, &context)?;
    act_on(session, understood, now_ms, voice)
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
        Understood::Intent(intent) => {
            println!("  understood   {:?}", intent.kind);
            if let Some(style) = &intent.style {
                println!("  style        {style}");
            }
            for (variable, target) in &intent.targets {
                println!("  target       {} {}", variable.0, describe(target));
            }

            let turn = session.request(&intent, now_ms)?;
            if turn.performed.is_empty() {
                println!("  result       nothing needed doing");
            }
            for invocation in &turn.performed {
                println!("  sent         {}", invocation.tool.as_str());
                println!("  accepted     by the appliance");
            }
        }
    }

    Ok(())
}

fn describe(target: &adi_protocol::intent::TargetSpec) -> String {
    use adi_protocol::intent::{Reference, TargetSpec};
    match target {
        TargetSpec::Absolute(q) => format!("= {} {:?}", q.value, q.unit),
        TargetSpec::Relative(r) => {
            let of = match &r.of {
                Reference::Last => "last".to_string(),
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
fn load_chooser(
    config_path: Option<&Path>,
    level: AutonomyLevel,
) -> Result<Option<(ProcessChooser, String)>, Box<dyn std::error::Error>> {
    if level == AutonomyLevel::D0 {
        return Ok(None);
    }

    let Some(path) = config_path else {
        return Ok(None);
    };

    let config = ModelsConfig::load(path)?;
    let seam = config.seam(Seam::Interpreter);

    // The chooser runs the same program as the interpreter. One model answers
    // both questions, which is what an appliance with one model resident
    // actually does.
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
    match ProcessChooser::start(&command, &seam.args, tag) {
        Ok(chooser) => Ok(Some((chooser, label))),
        Err(e) => {
            println!("  chooser      unavailable: {e}");
            Ok(None)
        }
    }
}

/// An interpreter and a description of where it came from, for the banner.
type Loaded = Option<(Box<dyn Interpreter>, String)>;

/// The interpreter seam, where it is configured.
///
/// A seam that cannot be made ready is reported and left unserved, for the
/// same reason the speech seams are: the session still has `cycle` and
/// `status`, which need no model, and ending it here would take those away
/// too. The caller already prints that spoken requests are unavailable.
fn load_interpreter(
    config_path: Option<&Path>,
    domain: &dyn adi_core::domain::Domain,
    device: &adi_protocol::declaration::Device,
) -> Result<Loaded, Box<dyn std::error::Error>> {
    match start_interpreter(config_path, domain, device) {
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
    device: &adi_protocol::declaration::Device,
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
            Ok(Some((Box::new(ScriptedInterpreter::load(&model)?), label)))
        }

        Backend::Process => {
            let command = seam.command.clone().ok_or("the process backend names no command")?;
            let label = format!("process ({command})");

            let knowledge = domain.knowledge();
            let interpreter = ProcessInterpreter::start(
                &command,
                &seam.args,
                Surroundings {
                    // With what each style is, not only its identifier. A
                    // request names a style the way a person would.
                    styles: knowledge
                        .styles_described()
                        .into_iter()
                        .map(|(name, description)| adi_language::context::StyleInfo {
                            name: name.to_string(),
                            description: description.map(str::to_string),
                        })
                        .collect(),
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
                                unit: declared.map(|d| format!("{:?}", d.unit).to_lowercase()),
                                meaning: declared.map(|d| d.description.trim().to_string()),
                            }
                        })
                        .collect(),
                    // The manufacturer's own material, so the machine can answer
                    // a question rather than only act on a request.
                    notes: knowledge.notes().cloned().collect(),
                    device: Some(format!(
                        "{} {}, firmware {}",
                        device.vendor, device.model, device.firmware_version
                    )),
                },
                Default::default(),
            )?;
            Ok(Some((Box::new(interpreter), label)))
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

    use super::{speech_rate, transcript_threshold, DEFAULT_SPEAKER_RATE, DEFAULT_TRANSCRIBER_RATE};
    use adi_models::{Backend, SeamConfig};

    fn seam(sample_rate: Option<u32>, min_confidence: Option<f64>) -> SeamConfig {
        SeamConfig {
            backend: Backend::Process,
            sample_rate,
            min_confidence,
            ..SeamConfig::none()
        }
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
