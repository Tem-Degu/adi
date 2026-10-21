# Models

Artificial Domain Intelligence reaches a model at four seams: the transcriber,
the interpreter, the explainer and the speaker. This document covers obtaining
the files, which ones were measured and what they scored, choosing hardware,
and using a model or a provider of your own.

The SDK itself is in [`README.md`](./README.md). Nothing here is required to
run the engine: see [Running without models](#running-without-models).

---

The SDK ships no model weights. They are obtained from the parties that publish
them, under those parties' licences, and placed in `models/`. There is no
downloader: this avoids redistributing third-party weights, keeps network code
out of a product that does not otherwise require it, and makes substituting a
different model the ordinary path rather than a special case.

## Obtaining them

```sh
adi models check --config models/models.yaml
```

This reads the folder and reports which seams are served, which files are
present, whether each matches a published checksum, and for anything absent,
the file to obtain and its source. It is generated from the folder rather than
written down, so it cannot describe a state the machine is not in.

`models/manifest.yaml` lists the recommended files with their sources,
licences and checksums. `models/models.yaml` states which file serves which
seam.

## Recommended models

**The interpreter seam needs a 4B class model or better, and the most capable
one the hardware will hold is the right choice.** This is not a performance
preference. A model too small for the job does not merely answer worse: the
1.7B we measured guessed where the correct answer was to ask a question, and a
guess becomes a physical action on a machine with a heating element in it. The
figures below say what each was measured doing.

All four seams are reached in a session. A cycle is explained and spoken, an
answer and a question are spoken, and `listen <file.wav>` transcribes a recorded
utterance and treats it as a request.

Speech output is written to `spoken.wav` rather than played, and input is read
from a file rather than captured. Playback and capture need an output device, a
microphone and a platform library, which is a packaging concern for a product
rather than part of the protocol. What the SDK demonstrates is that text reaches
the seam and audio comes back.

| Seam | Model | Licence | Status |
|---|---|---|---|
| Interpreter | Qwen 3 4B Q4_K_M, GGUF | Apache-2.0 | verified, see latency below |
| Transcriber | Moonshine base | MIT | verified |
| Speaker | Piper, en_GB-alba-medium | MIT | verified |

Verified means the shipped backend was run against the real weights and
produced correct output on this corpus. `models/manifest.yaml` carries the
checksums and what each was measured to do. Both checksums it states have been
confirmed against files freshly obtained from the published sources.

**On corpus accuracy, read this before quoting a figure.** Through the shipped
backend, `adi models bench --config models/models.example-local.yaml` scores
Qwen 3 4B Q4 at **7 of 18**, and a hosted model at **8 of 18**. Earlier
documents here quoted 9 of 18; that was measured with a harness written for the
occasion which was not kept, so it is not reproducible and is withdrawn.

Both figures rose by giving styles a description. A style reaches a model as an
identifier, and a request names one the way a person would, so mapping
"something short and strong" onto `adi.espresso.style.ristretto` was possible
only for a model that already knew the domain. Describing the four espresso
styles took the local model from 5 of 18 to 7 and the hosted one from 7 to 8,
which is the same reasoning that already applied to a variable's meaning and a
tool's description.

**A capable model scoring 8 of 18 is still a statement about the corpus.** Most
of its remaining misses are questions asked where the corpus expects an action,
and it is obeying the instruction when it asks: the instruction says not to
guess a style that was not stated, and several cases expect a style to be
inferred or omitted entirely. Nothing tells a model that `style` may be left
out, or when `adjust` is meant rather than `produce`. Until the instruction and
the corpus agree, this number measures that disagreement as much as it measures
a model. It is reported rather than quietly improved, because a measurement
nobody could reproduce is what produced the figure it replaces.

Qwen 3 1.7B is listed in the manifest and **not recommended**. On the shipped
corpus it reached 3 of 18, and it never asked a question where asking was the
correct answer. That is the wrong direction to fail in, because an ambiguous
request then becomes a physical action. It is kept in the manifest with this
warning rather than removed, so that someone who tries it learns why it should
not be used instead of finding out that it runs.

Note that 1.7B and 4B are both Qwen 3: the number before the B is the size and
the 3 is the generation. What was measured as unusable was the size.

Nothing between 4B and a hosted frontier model has been measured here, so the
supportable claim is that 4B is the smallest size found usable, not that it is
the best choice. Where a product can hold more, it should.

Llama models are not listed. The Llama licence requires permission from Meta
above a usage threshold and withholds multimodal rights from companies
domiciled in the European Union. A manufacturer licensing this SDK should not
inherit either condition.

## Selecting hardware

**Interpreter latency is the open problem in this SDK, and it is unsolved.**

Measured on four shared cloud cores with no accelerator, each backend kept
running as the engine keeps them:

| Seam | Model | Steady-state latency | Verdict |
|---|---|---|---|
| Speaker | Piper medium | **0.2 s** for 3 s of speech, 12x realtime | comfortable |
| Transcriber | Moonshine base | around 2 s for a short utterance | acceptable |
| Interpreter | Qwen 3 4B Q4 | **16 to 18 s median, 66 s at p95 or worse** | **unusable for speech** |
| Interpreter | a hosted model | **3.6 s median, 14 s at p95** | still above two seconds |

Reproduce either row with `adi models bench --config <file.yaml>`. The
interpreter rows are the whole corpus through the shipped backend, so the p95
is a real request and not an outlier to be discounted.

A person waiting on a machine tolerates perhaps two seconds. The interpreter is
an order of magnitude outside that, and the smaller model is half the latency
with a third of the accuracy, so it does not help.

What this means in practice:

- **The deterministic path is unaffected.** Observing a cycle, diagnosing it,
  planning a correction and enforcing the envelope involve no model. That runs
  in milliseconds and is what a machine does unattended.
- **Explaining is unaffected.** The explainer needs no model, and the speaker
  is fast.
- **Spoken requests are not yet viable** on a processor of this class, from a
  model running on it. An accelerator, a smaller task-specific model, or a
  different approach to the interpreter seam is required, and none of those has
  been measured. A hosted model is the fourth option and is available today;
  see [Using a hosted model](#using-a-hosted-model).

Measure on the board you intend to ship before committing to a voice
interface:

```sh
adi models bench --config models/models.example-local.yaml
```

This runs the corpus in `eval/espresso/` against the interpreter that
configuration names, on the host it is invoked on, and reports how often the
answer was correct and what each request cost. `--interpreter <file.yaml>`
measures the engine with the model time removed instead, which answers a
different and much smaller question.

Until recently this command could only do the second of those, while its own
output and three documents said otherwise. That is why the figures above are
stated with the command that reproduces them.

## Using a different model

Three levels, in increasing order of effort.

**Change the configuration.** Point `models/models.yaml` at a different file.
A model absent from the manifest loads normally and is recorded as unverified,
identified by its checksum. Verification exists so that a decision can be
attributed to a specific model afterwards, as ADI-P 9.3 requirement 5 requires,
not to restrict what may be loaded.

**Use another program.** The `process` backend runs an external program and
exchanges one JSON request and one JSON answer per line over its standard
input and output. It serves the interpreter, chooser, transcriber and speaker
seams.
The program requires no dependency on this SDK and may be written in any
language.

```
interpreter  -> {"utterance":"half of last time","instruction":"...",
                 "schema":{...},
                 "styles":[{"name":"...","description":"..."}],
                 "variables":[...],"materials":[...],
                 "notes":[...],"machine":"...","history":[...]}
             <- {"intent":{"kind":"produce","targets":{...}}}
             or {"clarify":{"question":"...","because":"..."}}
             or {"answer":{"text":"...","drew_on":["<note id>"]}}

chooser      -> {"goals":[{"variable":"shot_time","delta":-3.0,...}],
                 "candidates":[{"index":0,"tool":"...","variable":"shot_time",
                                "per_unit":-1.4,"current":24,...}],
                 "level":"d2","constraints":[...],"instruction":"...",
                 "schema":{...}}
             <- {"steps":[{"candidate":0,"value":21}]}

transcriber  -> {"audio":"<base64>","sample_rate":16000}
             <- {"text":"...","confidence":0.91}

speaker      -> {"text":"..."}
             <- {"audio":"<base64>","sample_rate":22050}
```

Audio is base64 of 16-bit signed, little endian, single channel samples.

A style carries what it is, as a variable carries its meaning and a tool its
description. A request names a style the way a person would, and an identifier
alone can be mapped onto one only by a model that already knows the domain,
which ADI-P 7.3 says a profile may not assume and which is never true of a
style a manufacturer added. A description may influence which style is chosen
and reaches no actuation value: the targets a style sets are what actuates, and
they are validated.

`instruction` and `schema` carry the contract itself. The engine owns what a
reply may be, narrows the schema to the styles, materials, variables and units
this machine declares, and sends both with every request, so a backend compiles
a grammar from what it was handed and holds no copy of its own. A copy is a
thing that drifts: the shipped Qwen backend carried one whose answer arm was
missing, and a machine running real weights met every question with a question
while the engine had permitted an answer throughout.

A backend may ignore both and validate nothing, since the engine validates
every reply regardless. What it may not do is compose a different contract and
expect the engine to honour it.

An interpreter has three outcomes rather than two. A request becomes an intent,
an unresolvable one becomes a question back, and a question from the person
becomes an answer, which actuates nothing. `machine` carries the device identity
and `notes` every note the knowledge base holds, so a backend needs no access to
either.

Notes are sent whole, on every utterance, with no relevance filter: selecting by
relevance guesses what a question needs and a wrong guess withholds the answer
silently. **Keeping the total within the context of the smallest model you
target is your responsibility.** The engine applies its budget, discards notes
before anything a request depends on, and prints a warning when it has to, so
exceeding the capacity is visible rather than silent.

Each candidate carries what its tool changes besides the outcome it was offered
for, as `also_affects`. That is what separates two tools serving one outcome:
changing the dose moves the strength as well, changing the grind does not, and
the magnitudes alone do not say so. It also carries a `description`, which is the
manufacturer's where they wrote one and the profile's for a registry tool.

A chooser answers with an index into the candidate list the engine supplied, so
it cannot name a tool the machine does not declare. `constraints` states the
rules the answer is checked against, including that no two steps may move the
same variable, so a backend author does not have to reproduce them. Where an
answer is refused for a reason the model could fix, the engine asks once more
with a `rejected` field explaining why. At D2 a value is the absolute setting to
send, not a change, which is why each candidate carries its `current` value.

Two sets of backends are supplied.

`examples/interpreters/reference.py` and
`examples/interpreters/reference_speech.py` implement the
protocol and contain no models. They are the specification by example, and are
used by the test suite.

`examples/interpreters/qwen_interpreter.py` and
`examples/interpreters/speech_backends.py` run real models, and are configured
by `models/models.example-local.yaml`. Both have been **run against real
weights** and corrected where they were wrong. See the table above for what
each was measured to do, and `models/manifest.yaml` for the checksums of the
files used.

Both also accept `--explain`, which reports how a request would be served and
loads no model. The test suite uses it, because a backend that runs real
weights cannot otherwise be tested where the weights are not present, and an
untested one is how a backend ships unable to answer at all.

`examples/interpreters/openai_interpreter.py` serves both model seams from a
hosted model. It needs no package beyond the standard library, reads the key
from the environment rather than from an argument, and carries the timeout the
next section asks for.

`examples/interpreters/adi_prompt.py` renders a request as a prompt and is
imported by the backends above. One copy, for the same reason the engine sends
the contract rather than asking each backend to remember it.

`models/models.example-process.yaml` configures the reference set.

## The two numbers on a speech seam

`sample_rate` is the rate the backend's model requires, not the rate the
microphone produces. Audio at any other rate is refused rather than resampled,
because audio read at the wrong rate does not error, it transcribes badly and
the model takes the blame. Moonshine wants 16000; Piper publishes voices at
both 16000 and 22050, so a speaker seam has to state which one it loaded.

`min_confidence` is the threshold below which a transcript is treated as not
heard, at which point the machine asks again instead of acting. ADI-P 9.4.3
makes asking an outcome rather than a failure. The default is 0.6, which is
deliberately not near-certain: speech recognition over a running grinder is
not a quiet-room benchmark, and the cost of asking again is a second of
someone's time against a wasted drink. Raise it for a noisier installation.

**Implement the trait.** `adi-language` defines the four seams as traits. A
manufacturer shipping at volume implements them directly against their own
runtime.

## Using a hosted model

ADI is designed to run fully locally. No account, no key, no network, and the
figures above were measured without an accelerator.

Where a network is available, a hosted model can serve any of the four seams.
The reason to use one is the interpreter latency above. A seam is a program that
reads one JSON request per line and writes one answer, so a script calling an
API is a complete implementation: no SDK dependency, any language, no change to
this engine.

Three things to get right.

**Put a timeout in your script.** The engine blocks waiting for a backend's
reply and imposes no limit of its own. A local model is slow and then finishes;
a stalled connection may never answer. While the engine waits it stops sending
heartbeats, so after `control_timeout_ms` the appliance decides the link is dead
and enters its safe state. That behaviour is correct and the cause will not be
obvious. Use a few seconds, and return a `clarify` when it expires.

**Constrain the output to the schema the engine sent.** Instruction is advice;
a grammar is not. The request carries `schema`, already narrowed to what this
machine declares, and `instruction`, so a backend does not compose either and
cannot drift from the contract the engine enforces. Hosted models take this as
structured output or a JSON schema.

Not every provider will apply it. The output schema offers an intent, a
question or an answer as a `oneOf` at the top level, which some refuse
outright; the selection schema is a plain object and is accepted.
`provider_can_enforce` in `examples/interpreters/openai_interpreter.py` shows
the test and states why the schema is not flattened to suit a dialect. Where
the provider will not apply it, state it in the prompt: weaker, and the engine
validates the reply either way.

**Decide what leaves the building.** On the speech seams that is audio of
whoever is in the room.

D0 uses no model at all, so none of this touches the deterministic path. A
machine with its network removed still corrects itself. It loses speech, not
intelligence.

### Every seam hosted, verified

`models/models.example-hosted.yaml` is this configuration, ready to use. It is
the counterpart to `models.example-local.yaml`.
`examples/interpreters/openai_speech.py` serves the transcriber and the
speaker, as `openai_interpreter.py` serves the interpreter and the chooser.
Between them a machine needs no model files, no ONNX runtime and nothing
downloaded, which is the quickest way to try the whole thing before committing
to hardware.

```yaml
seams:
  transcriber:
    backend: process
    command: python3
    args: ["examples/interpreters/openai_speech.py", "transcribe",
           "gpt-4o-transcribe"]
    sample_rate: 16000
    min_confidence: 0.6

  interpreter:
    backend: process
    command: python3
    args: ["examples/interpreters/openai_interpreter.py", "gpt-5.5"]

  speaker:
    backend: process
    command: python3
    args: ["examples/interpreters/openai_speech.py", "speak",
           "gpt-4o-mini-tts"]
    # The rate the provider returns for raw PCM. A reply at another rate is
    # refused rather than resampled.
    sample_rate: 24000
```

Run against the washing machine firmware on an emulated Cortex-M3, a spoken
request:

```
> listen spoken.wav
  heard        46068 samples at 16000 Hz
  transcript   "These towels came out soaking wet; spin them harder."  confidence 0.85
  understood   Adjust
  style        x.northwash.style.towels
  target       x.northwash.residual_moisture = now -10 Pct
  measured     90 Pct
  sent         x.northwash.spin.set_speed
               pct = 20 Pct
  accepted     by the appliance
```

and a spoken question, answered aloud:

```
> listen question.wav
  heard        28049 samples at 16000 Hz
  transcript   "How wet is the load right now?"  confidence 0.95
  answers      The load is at 80.0 pct residual moisture right now.
  spoken       112800 samples at 24000 Hz, written to spoken.wav
```

That answer is 4.7 seconds of speech at normal level, and the figure in it came
from the appliance's own sensor rather than from the model.

Audio crosses the seam as raw signed 16-bit little-endian mono samples, base64
encoded, in both directions. The provider wants a container on the way in and
returns raw PCM on the way out, so the backend wraps a WAV in memory and asks
for `pcm` rather than mp3, which avoids decoding something that did not need
encoding. Where the provider returns log probabilities the confidence comes
from them; where it does not, the backend falls back to the same estimate the
local one makes, so the threshold means one thing across both.

## Wiring a microphone and a speaker

The SDK handles sound once it is samples and once it has produced samples. Moving
those to and from a device is yours, because an output device, a microphone and
the library that drives them belong to a product and a platform rather than to a
protocol.

Two methods are the whole interface.

```rust
use adi_language::{Audio, Language};

// Build it once, from whichever backends your configuration names.
let language = Language::new(transcriber, interpreter, explainer, speaker);

// Someone spoke. Your capture code produced i16 mono samples.
let audio = Audio::new(samples, 16_000);
let understood = language.understand(&audio, &context)?;

// A decision was made. Hand the samples to your output device.
let (text, spoken) = language.report(&record)?;
```

`understand` transcribes, checks the result, and interprets in one step. A
transcript the backend is not confident in comes back as a question rather than
an intent, which matters more here than anywhere else: mishearing "hotter" as
"shorter" is a physical action on a machine with a heating element, and a
transcript is the one place where a wrong word is indistinguishable downstream
from a right one. The threshold is `min_transcript_confidence`, which defaults to
0.6 rather than something near certainty, because speech recognition over a
grinder is not a quiet-room benchmark and the cost of asking again is a second of
someone's time against a wasted drink.

What you supply:

**Samples in the rate the transcriber wants.** `transcriber.required_sample_rate()`
says which, and `understand` refuses a mismatch rather than resampling, since
resampling badly is worse than refusing. Signed 16 bit, mono.

**A decision about when to listen.** A button, a wake word, voice activity
detection. The SDK has no opinion and should not: a button on a commercial
machine and a wake word on a domestic one are different products, and neither
belongs in a protocol.

**Playback.** `report` returns the words alongside the samples, so a build with
no synthesis configured still has something to display.

What the SDK will not do for you is capture, playback, resampling, echo
cancellation or noise suppression. Those are the parts that need the device in
front of you, and no worked example against a real audio library is supplied yet.

### The path from a recording, verified

`adi run` takes `listen <file.wav>`, which is the same path a capture pipeline
takes, minus the capture. Signed 16 bit mono at the rate the transcriber asks
for, which for Moonshine is 16 kHz.

This was run against the washing machine firmware on an emulated Cortex-M3,
with Moonshine serving the transcriber locally and a hosted model serving the
interpreter:

```
> listen spoken.wav
  heard        46068 samples at 16000 Hz
  transcript   "These towels came out soaking wet, spin them harder."  confidence 0.97
  understood   Adjust
  style        x.northwash.style.towels
  target       x.northwash.residual_moisture = now -10 Pct
  measured     90 Pct
  sent         x.northwash.spin.set_speed
               pct = 20 Pct
  accepted     by the appliance
```

Every step there is a real one: the transcript came from the weights, the style
was recognised from the description its manufacturer wrote, the engine sized the
change from the reading the appliance published, and the value crossed the
framing, the checksum and the range check on the way to the drum.

The recording in that run was synthesised with the Piper voice rather than
spoken by a person, because the test ran on a machine with no microphone. To use
your own voice, record a sentence and convert it:

```sh
ffmpeg -i yours.m4a -ac 1 -ar 16000 -c:a pcm_s16le spoken.wav
```

A rate other than the one the transcriber asks for is refused rather than
resampled, so the conversion is not optional.

### The same, on every example machine

One spoken sentence per machine, run twice: once with Moonshine serving the
transcriber locally and once with every seam hosted. The two paths produced the
same action on each machine.

| Machine | What was said | What was sent |
|---|---|---|
| Example EX-900 espresso | "Make the next shot take thirty seconds" | `grind.set_setting` 23 step |
| Dryline DH-20 dehydrator | "Dry it down to twelve percent moisture" | `heater.set_temperature` 70 C, reported as partial at the declared limit |
| Rainline SX-2 shower | "Make it a bit warmer" | `mixer.set_temperature` 40 C, from a measured 38 |
| Northwash NW-8 washer | "These towels came out soaking wet, spin them harder" | `spin.set_speed` 20 pct, from a measured 90 |

Every transcript came back at 0.85 or better. Three of the four machines are
firmware on an emulated Cortex-M3 reached over a UART; the espresso machine is
the same library built for a host.

One run is worth describing because it looks like a failure and is not. Asked a
second time inside two minutes, the dehydrator refused: its declaration states
a 120 second interval between calls to the heater, and the appliance enforces
that itself. The engine reported what the machine said and the session carried
on.

### The same, with nothing hosted

The four machines again, with the 4B interpreter running locally beside
Moonshine and Piper: no key, no network, nothing leaving the building. On four
shared cloud cores, no accelerator.

| Machine | Heard | Understood | Outcome | Wall clock |
|---|---|---|---|---|
| Espresso | exact | "half of last time", from a sentence stating thirty seconds | refused, nothing remembered to be half of | 2m04s |
| Dehydrator | exact | a change against the present reading, approximately right | sent 38 C, accepted | 3m14s |
| Shower | exact | correct | refused: the chooser proposed the setting already in force | 3m51s |
| Washer | exact | correct | stopped: the appliance faulted on the heartbeat gap | 3m54s |

Read that as three separate statements.

**Hearing is not the weak part.** Moonshine transcribed all four at 0.97,
matching the hosted transcriber.

**The path is mechanically complete.** Audio becomes a transcript, a transcript
becomes an intent, the engine plans from a reading the appliance published, and
on the dehydrator the value reached the element and was accepted. Every failure
came from a component entitled to refuse, and each said why.

**It is not usable at this speed, and the appliances are right to say so.** Two
to four minutes an utterance, against declarations that allow four seconds
between heartbeats. While the engine waits for a model it sends none, so the
appliance enters its safe state and aborts what the engine started. That is the
declaration being enforced rather than a defect, and it is the reason to
measure latency on the hardware a product will actually ship, which the
interpreter entry in `manifest.yaml` also says.

Two model faults are worth naming because the engine caught both. Given a
sentence with a number in it, this model twice produced a relation to a past
cycle instead; the instruction now shows an absolute example and it still did
so on the espresso sentence. And at D2 it proposed the value already in force,
which the engine refuses because a correction that moves nothing is not a
correction. D0 and D1 are unaffected by the second of those, since the engine
computes the value at both.

One cost of ours, found here and not yet addressed: at D1 and D2 the engine
starts the chooser as its own process with the same command as the
interpreter, so two processes serve what the code's own comment describes as
one model answering both questions.

Measured on this machine, with the page cache dropped between the two runs, it
is not the doubling it appears to be. `llama.cpp` maps the GGUF, so both
processes read the same physical pages: `Pss_File` is 2428 MB in a single
process and 1216 MB in each of two, which is the same weights counted twice.
What is not shared is 2092 MB of private anonymous memory per process, the
compute and cache buffers, including tensors repacked into the form the CPU
backend wants. So the second seam costs about 2.1 GB more rather than another
4.5 GB: `free` reported 2.25 GB of anonymous use with one process and 4.2 GB
with two, over a 2.5 GB page cache either way.

Whether it is worth changing depends on where the model runs, and on this
engine it is not urgent either way.

On a hosted seam the duplicate costs 20 MB, measured, because the process holds
no weights. On a local model it costs about 2.1 GB.

Nothing is gained by the second process today, either. The engine's loop reads
an utterance, interprets it, chooses, invokes, and only then reads again, so
the two seams are never in use at the same moment. Within one utterance they
cannot be: choosing a mechanism requires knowing the intent. They could only
overlap across utterances, which needs a concurrent engine loop first, and on
one local model that concurrency is largely illusory, since two inferences on
the same cores halve each other rather than running in parallel. On a hosted
seam the work happens elsewhere, so two pipes would give real parallelism to an
engine that wanted it.

So the time to make the seams share one process is when a local model is fast
enough to ship, which is the same point at which memory becomes the binding
constraint, and the sharing should be conditional on both seams naming the same
command so that two programs remain possible. The backends already read the
contract off each request, so nothing in the protocol stands in the way.

## Running without models

An engine with an unserved seam is a supported configuration. It refuses the
operations that seam provides and names the seam, rather than failing. The
explainer works without any model, producing text from the decision record.

---
