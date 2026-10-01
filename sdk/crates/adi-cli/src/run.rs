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
use adi_domain_espresso::Espresso;
use adi_language::context::VariableInfo;
use adi_language::{Explainer, RecordExplainer, Speaker, Transcriber};
use adi_models::{
    ProcessSpeaker, ProcessTranscriber, Surroundings,
    Backend, ModelsConfig, ProcessChooser, ProcessInterpreter, ScriptedInterpreter, Seam,
};
use adi_protocol::declaration::Declaration;
use adi_protocol::ids::VariableId;
use adi_protocol::skill::Skill;
use adi_protocol::units::{Quantity, Unit};
use adi_session::{ChildAppliance, Session};

pub fn run(
    declaration_path: PathBuf,
    skill_path: PathBuf,
    appliance_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    level: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let declaration: Declaration = serde_yaml::from_str(&read(&declaration_path)?)?;
    let skill: Skill = serde_yaml::from_str(&read(&skill_path)?)?;
    let domain = Espresso::new();

    // Checked before anything is started. An argument that cannot be honoured
    // should be refused on its own terms, not after a failure to find a
    // binary that was never going to be used.
    let requested = parse_level(level.as_deref())?;

    let appliance_path = appliance_path
        .map(|given| resolve_executable(&given))
        .unwrap_or_else(default_appliance);

    if !appliance_path.exists() {
        return Err(format!(
            "no appliance at {}\n\nBuild it first:\n    cd sdk/c && make example",
            appliance_path.display()
        )
        .into());
    }

    println!(
        "  machine      {} {}",
        declaration.device.vendor, declaration.device.model
    );
    println!("  appliance    {}", appliance_path.display());

    let transport = ChildAppliance::start(&appliance_path.to_string_lossy(), &[])?;

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
                &domain,
                Box::new(selector),
                Box::new(transport),
            )
        }
        (AutonomyLevel::D1, Some((chooser, label))) => {
            println!("  chooser      {label}");
            Session::open_with_selector(
                &declaration,
                Some(&skill),
                &domain,
                Box::new(ModelToolSelector { chooser }),
                Box::new(transport),
            )
        }
        (AutonomyLevel::D0, _) => Session::open(
            &declaration,
            Some(&skill),
            &domain,
            AutonomyLevel::D0,
            Box::new(transport),
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

    let interpreter = load_interpreter(config_path.as_deref(), &domain, &declaration.device)?;
    let voice = Voice::load(config_path.as_deref())?;
    match &interpreter {
        Some((_, label)) => println!("  interpreter  {label}"),
        None => println!("  interpreter  none, so spoken requests are unavailable"),
    }

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
            "cycle" => cycle(&mut session, now_ms, &voice),
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
}

impl Voice {
    /// Start whichever of the two the configuration names.
    fn load(config_path: Option<&Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let Some(path) = config_path else {
            return Ok(Self {
                speaker: None,
                transcriber: None,
            });
        };

        let config = ModelsConfig::load(path)?;

        let speaker = {
            let seam = config.seam(Seam::Speaker);
            match (seam.backend, seam.command.clone()) {
                (Backend::Process, Some(command)) => Some(Box::new(ProcessSpeaker::start(
                    &command,
                    &seam.args,
                    22_050,
                )?) as Box<dyn Speaker>),
                _ => None,
            }
        };

        let transcriber = {
            let seam = config.seam(Seam::Transcriber);
            match (seam.backend, seam.command.clone()) {
                (Backend::Process, Some(command)) => Some(Box::new(ProcessTranscriber::start(
                    &command,
                    &seam.args,
                    16_000,
                )?) as Box<dyn Transcriber>),
                _ => None,
            }
        };

        Ok(Self {
            speaker,
            transcriber,
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
fn cycle(
    session: &mut Session<'_>,
    now_ms: u64,
    voice: &Voice,
) -> Result<(), Box<dyn std::error::Error>> {
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

    let transcript = transcriber.transcribe(&audio)?;
    println!(
        "  transcribed  \"{}\" (confidence {:.2})",
        transcript.text, transcript.confidence
    );

    // From here it is an ordinary spoken request. A transcript is text, and
    // nothing downstream needs to know it arrived as sound.
    request(session, interpreter, &transcript.text, now_ms, voice)
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

    match interpreter.interpret(words, &context)? {
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

    Ok(Some((
        ProcessChooser::start(&command, &seam.args, tag)?,
        label,
    )))
}

/// An interpreter and a description of where it came from, for the banner.
type Loaded = Option<(Box<dyn Interpreter>, String)>;

/// Build whatever interpreter the configuration names.
fn load_interpreter(
    config_path: Option<&Path>,
    domain: &Espresso,
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

            use adi_core::Domain;
            let knowledge = domain.knowledge();
            let interpreter = ProcessInterpreter::start(
                &command,
                &seam.args,
                Surroundings {
                    styles: knowledge.style_ids().into_iter().map(str::to_string).collect(),
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
