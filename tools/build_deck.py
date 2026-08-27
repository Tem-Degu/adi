# -*- coding: utf-8 -*-
"""Full StreetAI / Artificial Domain Intelligence deck.

The complete version, including the architecture, Safe Action Protocol, the
autonomy levels and the integration mechanics. For the investor cut, which
drops those and adds market and money, see `build_deck_investor.py`.

Register: technical report. See the copy rules in `deck_kit.py`.

The .pptx is GENERATED. Edit this script, not the file. If someone has started
editing the deck in PowerPoint, patch the file instead or their work is lost.

Run:  python tools/build_deck.py
"""

from pathlib import Path

from pptx.util import Inches
from pptx.enum.text import PP_ALIGN

from deck_kit import (
    Deck, M, CW, CW2, CW3, G3, CW4, G4,
    TEXT, MUTED, ACCENT, TEAL, CARD, CARD_HI,
)

ASSETS = Path(__file__).resolve().parent.parent / "assets"

d = Deck()

# ---------------------------------------------------------------- 01 Title
s = d.slide(footer=False, number=False)
d.bar(s, M, Inches(2.05), Inches(1.3), Inches(0.06), ACCENT)
d.tb(s, "StreetAI", M, Inches(2.45), Inches(6.7), Inches(0.5), 15, MUTED,
     bold=True, caps=True)
d.tb(s, "Artificial\nDomain Intelligence", M, Inches(2.95), Inches(6.7),
     Inches(2.0), 54, TEXT, bold=True, line=1.04)
d.picture(s, ASSETS / "appliances.jpg", M + Inches(6.9), Inches(2.62),
          Inches(4.73), Inches(2.58))
d.tb(s, "An embedded intelligence layer licensed to appliance manufacturers.",
     M, Inches(5.32), Inches(6.7), Inches(0.45), 17, ACCENT, line=1.3)
d.tb(s, "First domain: espresso.", M, Inches(6.0), Inches(6.7), Inches(0.32),
     16, MUTED)
d.tb(s, "Technical and commercial brief  ·  2026", M, Inches(6.45), Inches(6),
     Inches(0.3), 12, MUTED, caps=True)

# ---------------------------------------------------------------- 02 Problem
s = d.slide()
d.heading(s, "Problem", "Current AI systems have no\nphysical interface.")
d.bullets(s, [
    "Large language models perform well on text tasks such as drafting, code and professional examinations.",
    "They have no sensors, no actuators and no knowledge of any particular machine.",
    "Household appliances run fixed programs. They do not measure the outcome and do not adjust.",
    "Approximately 850 million major home appliances ship each year. Almost none of them evaluate their own output.",
], M, Inches(3.2), Inches(10.6), Inches(2.6), 17, gap=16)

# ---------------------------------------------------------------- 03 Position
s = d.slide()
d.heading(s, "Position", "Between conversational AI\nand general robotics.")
y = Inches(3.05)
d.card(s, M, y, CW3, Inches(2.6), "Conversational AI",
       "ChatGPT, Claude, Gemini. High capability on language tasks. No physical "
       "sensing or actuation.", MUTED)
d.card(s, M + CW3 + G3, y, CW3, Inches(2.6), "Artificial Domain Intelligence",
       "Bounded intelligence embedded in one machine. Runs on current appliance "
       "hardware at current component cost.", ACCENT)
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.6), "General robotics",
       "Humanoid and general-purpose systems. Safety and unit cost remain "
       "unsolved.", MUTED)
d.tb(s, "Investment is concentrated at both ends of this range.",
     M, Inches(6.0), Inches(11.2), Inches(0.4), 15, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 04 Definition
s = d.slide()
d.heading(s, "Definition", "Artificial Domain Intelligence.")
d.tb(s, "It gives a single-purpose machine real intelligence within its own "
        "domain. A domain is the scope of what the machine is for.",
     M, Inches(2.45), Inches(10.6), Inches(1.0), 21, TEXT, line=1.32)
y = Inches(3.9)
for i, (t, b) in enumerate([
    ("Perceive", "It reads the physical state of what it is working on."),
    ("Reason", "It applies expert knowledge of one job to decide what to do."),
    ("Learn", "It adapts to how this owner likes things done."),
    ("Take instruction", "It understands plain speech and accepts correction."),
]):
    d.card(s, M + i * (CW4 + G4), y, CW4, Inches(2.15), t, b, ACCENT, 15.5, 12.5)
d.tb(s, "The intelligence is bounded to one domain. It is not general intelligence.",
     M, Inches(6.3), Inches(11), Inches(0.4), 15, MUTED)

# ---------------------------------------------------------------- 05 Bounded
s = d.slide()
d.heading(s, "Why bounded intelligence ships", "Five consequences of a narrow domain.")
d.bullets(s, [
    ("Model size.  ", "A single task requires a small model, which runs on the device."),
    ("No network.  ", "There is no cloud dependency, user account or subscription."),
    ("Testability.  ", "A narrow decision space can be enumerated, bounded and tested exhaustively."),
    ("Certification.  ", "Behaviour can be demonstrated to regulators and insurers."),
    ("Component cost.  ", "The silicon required is within the bill of materials of a premium appliance."),
], M, Inches(2.9), Inches(10.9), Inches(3.3), 17, gap=18)

# ---------------------------------------------------------------- 06 Enablers
s = d.slide()
d.heading(s, "Enabling conditions", "Three constraints lifted since 2023.")
y = Inches(3.1)
d.card(s, M, y, CW3, Inches(2.6), "Model size",
       "Language models at 1 to 3 billion parameters reach usable accuracy on "
       "constrained tasks and fit in appliance-class memory.")
d.card(s, M + CW3 + G3, y, CW3, Inches(2.6), "On-device inference",
       "Runtimes and neural accelerators for embedded hardware are established "
       "engineering rather than research.")
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.6), "Component cost",
       "Sensors and inference silicon are priced in single-digit dollars.")

# ---------------------------------------------------------------- 07 Architecture
s = d.slide()
d.heading(s, "Architecture", "Four layers, reused across domains.")
y, rh = Inches(2.5), Inches(0.80)
for i, (n, t, b) in enumerate([
    ("01", "Perception", "Sensors report the physical state of the process."),
    ("02", "Domain reasoning", "Process knowledge applied within declared safety limits."),
    ("03", "Language interface", "Speech and text for instruction and correction."),
    ("04", "Actuation", "A bounded control interface to motors, heaters and valves."),
]):
    yy = y + i * (rh + Inches(0.14))
    d.rect(s, M, yy, Inches(11.0), rh)
    d.tb(s, n, M + Inches(0.34), yy + Inches(0.24), Inches(0.6), Inches(0.4),
         15, ACCENT, bold=True)
    d.tb(s, t, M + Inches(1.05), yy + Inches(0.22), Inches(2.7), Inches(0.4),
         16.5, TEXT, bold=True)
    d.tb(s, b, M + Inches(3.9), yy + Inches(0.24), Inches(6.9), Inches(0.45),
         13.5, MUTED)
d.tb(s, "Moving to a new domain replaces the perception adapters, the knowledge "
        "base and the actuation interface. The remainder is unchanged.",
     M, Inches(6.3), Inches(11), Inches(0.5), 14.5, MUTED, line=1.35)

# ---------------------------------------------------------------- 08 Action set
s = d.slide()
d.heading(s, "Action interface", "The machine declares what\nit can do.")
d.tb(s, "Set grind. Dose by mass. Start extraction. Stop at weight. Dispense "
        "milk. Eject. The intelligence acts by invoking declared actions.",
     M, Inches(2.75), Inches(10.6), Inches(0.9), 17, TEXT, line=1.35)
y = Inches(3.85)
d.card(s, M, y, CW3, Inches(2.3), "Declaration",
       "The manufacturer publishes each action its machine supports, with "
       "parameters and limits, as a data file.")
d.card(s, M + CW3 + G3, y, CW3, Inches(2.3), "Selection",
       "The intelligence reads the declaration and determines which actions "
       "satisfy the user's request.", ACCENT)
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.3), "Boundary",
       "An action that was not implemented and declared cannot be invoked.")
d.tb(s, "This is the mechanism by which language models already operate on "
        "software tools, applied to a physical machine.",
     M, Inches(6.3), Inches(11.2), Inches(0.5), 14.5, MUTED,
     align=PP_ALIGN.CENTER, line=1.35)

# ---------------------------------------------------------------- 09 Safe Action
s = d.slide()
d.heading(s, "Safe Action Protocol", "Two levels of constraint.")
y = Inches(2.55)
d.card(s, M, y, CW2, Inches(2.05), "Per invocation",
       "Parameters within declared range and inside every hard limit. Verified "
       "independently by the machine's own firmware.", ACCENT, 17, 13)
d.card(s, M + CW2 + Inches(0.3), y, CW2, Inches(2.05), "Across a sequence",
       "Container capacity, thermal recovery interval and consumable level. "
       "Constraints that exist only over time.", TEAL, 17, 13)
d.rect(s, M, Inches(4.78), Inches(11.0), Inches(1.45), CARD_HI)
d.tb(s, "Three dispense actions of 150 ml each are individually within range.",
     M + Inches(0.4), Inches(5.06), Inches(10.2), Inches(0.4), 16, TEXT, bold=True)
d.tb(s, "Their sum exceeds a 320 ml container. The engine rejects the third and "
        "reports the constraint that would have been breached.",
     M + Inches(0.4), Inches(5.48), Inches(10.2), Inches(0.5), 14.5, MUTED, line=1.35)
d.tb(s, "The language model does not participate in actuation at any level.",
     M, Inches(6.44), Inches(11.2), Inches(0.4), 14.5, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 10 Autonomy
s = d.slide()
d.heading(s, "Autonomy levels", "Configurable per integration.")
y, rh = Inches(2.65), Inches(0.95)
for i, (lvl, a, b, note, col) in enumerate([
    ("D0", "Engine selects action", "Engine computes value",
     "Fully reproducible. Applicable where certification requires it.", MUTED),
    ("D1", "Model selects action", "Engine computes value",
     "Supports actions outside the standard set. Current configuration.", ACCENT),
    ("D2", "Model selects action", "Model computes value",
     "Requires sequence constraints. Refuses to initialise without them.", MUTED),
]):
    yy = y + i * (rh + Inches(0.16))
    d.rect(s, M, yy, Inches(11.0), rh, CARD_HI if i == 1 else CARD)
    d.tb(s, lvl, M + Inches(0.34), yy + Inches(0.3), Inches(0.8), Inches(0.4),
         19, col, bold=True)
    d.tb(s, a, M + Inches(1.3), yy + Inches(0.32), Inches(2.9), Inches(0.4), 14, TEXT)
    d.tb(s, b, M + Inches(4.3), yy + Inches(0.32), Inches(2.6), Inches(0.4), 14, TEXT)
    d.tb(s, note, M + Inches(7.1), yy + Inches(0.3), Inches(3.6), Inches(0.55),
         12.5, MUTED, line=1.3)
d.tb(s, "The level is a configuration setting. The safety constraints apply at all three.",
     M, Inches(6.2), Inches(11.2), Inches(0.4), 15, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 11 Domain
s = d.slide()
d.heading(s, "First domain", "Espresso extraction.")
d.bullets(s, [
    "Extraction quality depends on grind size, dose, water temperature, pressure and time. These variables interact.",
    "Users converge on settings by trial and error, typically 5 to 10 attempts for each new coffee.",
    "Bean properties change with age, so settings require repeated correction.",
    "The machine measures none of these outcomes and retains no history between cycles.",
], M, Inches(2.9), Inches(10.6), Inches(2.6), 17, gap=16)
d.tb(s, "The domain was selected because the sensing is tractable and each cycle "
        "completes in under a minute.",
     M, Inches(5.85), Inches(10.8), Inches(0.6), 15, MUTED, line=1.35)

# ---------------------------------------------------------------- 12 Behaviour
s = d.slide()
d.heading(s, "Behaviour", "Measured cycle, diagnosis, correction.")
rx = M + CW2 + Inches(0.3)
d.rect(s, M, Inches(2.6), CW2, Inches(3.4), CARD)
d.bar(s, M + Inches(0.34), Inches(2.94), Inches(0.5), Inches(0.045), MUTED)
d.tb(s, "Without the intelligence", M + Inches(0.34), Inches(3.18), CW2 - Inches(0.68),
     Inches(0.4), 17, TEXT, bold=True)
d.bullets(s, ["The user selects settings by estimation.",
              "The machine executes without measurement.",
              "No result is retained.",
              "Each new coffee restarts the process."],
          M + Inches(0.34), Inches(3.78), CW2 - Inches(0.68), Inches(2.0), 14,
          MUTED, gap=10, marker_color=MUTED)
d.rect(s, rx, Inches(2.6), CW2, Inches(3.4), CARD_HI)
d.bar(s, rx + Inches(0.34), Inches(2.94), Inches(0.5), Inches(0.045), ACCENT)
d.tb(s, "With the intelligence", rx + Inches(0.34), Inches(3.18), CW2 - Inches(0.68),
     Inches(0.4), 17, TEXT, bold=True)
d.bullets(s, ['User: "18 grams in, light Ethiopian."',
              'System: "36 g in 22 seconds, reported sour. Reduce grind by two steps."',
              'System: "28 seconds, reported balanced. Store as a recipe?"',
              'User: "Make my usual." The recipe is executed.'],
          rx + Inches(0.34), Inches(3.78), CW2 - Inches(0.68), Inches(2.1), 14,
          TEXT, gap=10)
d.tb(s, "Convergence in 2 to 3 cycles in place of 5 to 10.",
     M, Inches(6.25), Inches(11), Inches(0.4), 15, ACCENT, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 13 UX
s = d.slide()
d.heading(s, "User-visible behaviour", "No application and no account.")
d.bullets(s, [
    "Nothing is installed, registered or configured. The intelligence is part of the machine.",
    "It identifies channeling from flow data and attributes the fault to preparation rather than to grind size.",
    "Target values shift toward the user's rated preferences over successive cycles.",
    "Settings are corrected as bean age changes the extraction rate.",
    "All processing is local. The machine does not require a network connection.",
], M, Inches(2.9), Inches(10.8), Inches(3.2), 17, gap=16)

# ---------------------------------------------------------------- 14 Integration
s = d.slide()
d.heading(s, "Integration", "Manufacturer scope of work.")
y = Inches(3.2)
d.card(s, M, y, CW3, Inches(2.5), "Two data files",
       "One declaring the available actions and their limits. One describing "
       "how the machine responds. Neither requires programming.")
d.card(s, M + CW3 + G3, y, CW3, Inches(2.5), "Action handlers",
       "Implemented in the manufacturer's existing firmware, in their own "
       "language and on their own schedule.")
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.5), "Extension",
       "Actions outside the standard set operate as soon as they are declared, "
       "with no change to the engine.", ACCENT)
d.tb(s, "No part of adding functionality depends on a release from StreetAI.",
     M, Inches(6.1), Inches(11.2), Inches(0.4), 15, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 15 Portability
s = d.slide()
d.heading(s, "Domain portability", "Espresso is the first\nimplementation.")
y = Inches(2.42)
d.picture(s, ASSETS / "espresso-machine.jpg", M, y, Inches(6.4), Inches(3.49))
rx = M + Inches(6.7)
d.panel(s, rx, y, Inches(4.3), Inches(1.65), "Replaced per domain",
        "Perception adapters, the process knowledge base, and the "
        "actuation interface.", MUTED)
d.panel(s, rx, y + Inches(1.84), Inches(4.3), Inches(1.65), "Reused",
        "Reasoning engine, preference learning, language interface, safety "
        "layer, storage and data handling.", ACCENT)
d.tb(s, "Candidate domains: ovens, laundry, HVAC, controlled-environment "
        "agriculture, aquaria, additive manufacturing, laboratory equipment.",
     M, Inches(6.16), Inches(11.2), Inches(0.4), 13.5, TEAL, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 16 Landscape
s = d.slide()
d.heading(s, "Competitive landscape", "No comparable licensed layer.")
y, rh = Inches(2.45), Inches(0.70)
d.tb(s, "Category", M + Inches(0.3), y - Inches(0.36), Inches(2.5), Inches(0.3),
     11, MUTED, bold=True, caps=True)
d.tb(s, "Examples", M + Inches(3.0), y - Inches(0.36), Inches(3.0), Inches(0.3),
     11, MUTED, bold=True, caps=True)
d.tb(s, "Limitation", M + Inches(6.3), y - Inches(0.36), Inches(4.5),
     Inches(0.3), 11, ACCENT, bold=True, caps=True)
for i, (a, b, c) in enumerate([
    ("Smart appliances", "LG, Samsung, Miele", "Sensor thresholds and preset cycles. Proprietary to one brand."),
    ("Voice assistants", "Alexa, Google", "General language capability with no model of the appliance."),
    ("Embedded ML tools", "Edge Impulse, TFLite", "Development toolkits. The application is built by the customer."),
    ("Small models", "Llama, Phi, Gemma", "A component. Requires the surrounding system to be built."),
    ("Smart espresso", "Decent, Meticulous", "Integrated products. Demonstrates demand. Not licensable."),
]):
    yy = y + i * (rh + Inches(0.09))
    d.rect(s, M, yy, Inches(11.0), rh)
    d.tb(s, a, M + Inches(0.3), yy + Inches(0.2), Inches(2.6), Inches(0.4), 14,
         TEXT, bold=True)
    d.tb(s, b, M + Inches(3.0), yy + Inches(0.21), Inches(3.2), Inches(0.4),
         12.5, MUTED)
    d.tb(s, c, M + Inches(6.3), yy + Inches(0.21), Inches(4.5), Inches(0.4),
         12.5, MUTED)
d.tb(s, "Each manufacturer currently develops its own version independently.",
     M, Inches(6.45), Inches(11.2), Inches(0.4), 14, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 17 Product
s = d.slide()
d.heading(s, "Product", "Three components.")
y = Inches(3.05)
d.card(s, M, y, CW3, Inches(2.6), "Compute module",
       "Built to StreetAI specification on qualified third-party silicon, "
       "pre-certified, supplied under the StreetAI mark.")
d.card(s, M + CW3 + G3, y, CW3, Inches(2.6), "Software",
       "The reasoning engine, safety layer, domain knowledge, preference "
       "learning and the integration standard.")
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.6), "Certification mark",
       "A consumer-visible mark applied to the manufacturer's product.")
d.tb(s, "Silicon is sourced externally. The layer above it is proprietary.",
     M, Inches(6.05), Inches(11.2), Inches(0.4), 15, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 18 Revenue
s = d.slide()
d.heading(s, "Revenue model", "Four lines, one recurring.")
y = Inches(2.95)
for i, (t, b) in enumerate([
    ("Integration fee", "One-time payment covering the engineering to bring a manufacturer's first product to market."),
    ("Per-unit royalty", "A fee on each machine shipped, for the production life of the model."),
    ("Module margin", "Hardware margin on the compute module."),
    ("Updates", "Annual subscription for revised models and additional domain knowledge."),
]):
    d.card(s, M + i * (CW4 + G4), y, CW4, Inches(2.8), t, b, ACCENT, 16, 12.5)
d.tb(s, "Appliance models remain in production for five to seven years, so a "
        "single integration produces royalty income across that period.",
     M, Inches(6.05), Inches(11.2), Inches(0.5), 15, MUTED,
     align=PP_ALIGN.CENTER, line=1.35)

# ---------------------------------------------------------------- 19 Sequence
s = d.slide()
d.heading(s, "Market entry", "Premium segment first.")
y, rh = Inches(2.75), Inches(1.05)
for i, (p, t, b) in enumerate([
    ("Phase 1", "Premium", "Appliances priced $500 to $3,000. Margin absorbs first-generation component cost. Target a single manufacturer with a short development cycle."),
    ("Phase 2", "Mid-market", "Component cost falls and the module matures. The feature moves from flagship option to standard specification."),
    ("Phase 3", "Volume", "Cost becomes immaterial relative to unit price. Licensing extends across appliance categories."),
]):
    yy = y + i * (rh + Inches(0.16))
    d.rect(s, M, yy, Inches(11.0), rh, CARD if i else CARD_HI)
    d.tb(s, p, M + Inches(0.34), yy + Inches(0.2), Inches(1.2), Inches(0.3),
         11.5, ACCENT if i == 0 else MUTED, bold=True, caps=True)
    d.tb(s, t, M + Inches(0.34), yy + Inches(0.48), Inches(2.4), Inches(0.4),
         19, TEXT, bold=True)
    d.tb(s, b, M + Inches(3.2), yy + Inches(0.24), Inches(7.5), Inches(0.75),
         13.5, MUTED, line=1.3)

# ---------------------------------------------------------------- 20 Moat
s = d.slide()
d.heading(s, "Defensibility", "Four sources.")
d.bullets(s, [
    ("Integration standard.  ", "Once a manufacturer has implemented against the interface, replacement costs them a full product cycle."),
    ("Domain knowledge.  ", "Each domain encodes accumulated process expertise. It is independent of the hardware it runs on."),
    ("Field data.  ", "Deployed units generate measurements that improve subsequent models."),
    ("Certification mark.  ", "A consumer-visible mark creates demand from the manufacturer's own customers."),
], M, Inches(2.9), Inches(11.0), Inches(3.2), 16.5, gap=18)

# ---------------------------------------------------------------- 21 Status
s = d.slide()
d.heading(s, "Current status", "Specification and engine complete.")
y = Inches(2.75)
d.card(s, M, y, CW3, Inches(2.5), "Integration specification",
       "The complete contract a manufacturer implements, covering capability "
       "declaration, safety limits and execution.", ACCENT)
d.card(s, M + CW3 + G3, y, CW3, Inches(2.5), "Reasoning and safety software",
       "Builds and passes its test suite against example machine definitions.",
       ACCENT)
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.5), "Espresso domain",
       "Implemented on the engine: measurement, diagnosis, fault detection and "
       "correction.", ACCENT)
d.tb(s, "Outstanding work is hardware integration, trials with users, and a "
        "manufacturer agreement.",
     M, Inches(5.55), Inches(11.2), Inches(0.4), 15, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 22 Ask
s = d.slide(number=False)
d.bar(s, M, Inches(1.9), Inches(1.3), Inches(0.06), ACCENT)
d.tb(s, "Next stage", M, Inches(2.3), CW, Inches(0.4), 12, MUTED, bold=True, caps=True)
d.tb(s, "Reach the first\nmanufacturer agreement.", M, Inches(2.75), Inches(11),
     Inches(1.3), 38, TEXT, bold=True, line=1.1)
d.bullets(s, [
    "Validate the espresso product on production machines with users.",
    "Publish the integration standard and secure implementations against it.",
    "Produce the reference compute module.",
    "Complete the first manufacturer agreement.",
], M, Inches(4.5), Inches(9.5), Inches(2.0), 16.5, gap=13)

d.save(r"C:\T-Works\domain-intelligence\StreetAI-Artificial-Domain-Intelligence-Deck.pptx")
