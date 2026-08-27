# -*- coding: utf-8 -*-
"""Investor cut of the StreetAI deck.

Differs from the full deck by removing the architecture, the protocol, the
autonomy levels and the integration mechanics, and adding market, unit
economics, traction, team and use of funds.

Register: technical report. See the copy rules in `deck_kit.py`.

PLACEHOLDERS the founder must fill before sending. Search for "[[" in the
output, or run `python tools/check_deck.py`:
  - "Team": names and roles.
  - "Use of funds": the raise amount and the split.

MARKET FIGURES are sourced on the slide. Published estimates for the same
market differ by a factor of two or more between research firms, so the deck
states a range and names the source rather than asserting one number.

Run:  python tools/build_deck_investor.py
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
d.tb(s, "First domain: espresso.",
     M, Inches(6.0), Inches(6.7), Inches(0.32), 16, MUTED)
d.tb(s, "Investor deck  ·  2026", M, Inches(6.45), Inches(6), Inches(0.3), 12,
     MUTED, caps=True)

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
       "Bounded intelligence embedded in one machine. Runs on current "
       "appliance hardware at current component cost.", ACCENT)
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

# ---------------------------------------------------------------- 05 Enablers
s = d.slide()
d.heading(s, "Enabling conditions", "Three constraints lifted since 2023.")
y = Inches(3.1)
d.card(s, M, y, CW3, Inches(2.6), "Model size",
       "Language models at 1 to 3 billion parameters now reach usable accuracy "
       "on constrained tasks, and fit in appliance-class memory.")
d.card(s, M + CW3 + G3, y, CW3, Inches(2.6), "On-device inference",
       "Runtimes and neural accelerators for embedded hardware are established "
       "engineering rather than research.")
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.6), "Component cost",
       "Sensors and inference silicon are priced in single-digit dollars, "
       "within the bill of materials of a premium appliance.")

# ---------------------------------------------------------------- 06 Domain
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

# ---------------------------------------------------------------- 07 Behaviour
s = d.slide()
d.heading(s, "Behaviour", "Measured cycle, diagnosis, correction.")
rx = M + CW2 + Inches(0.3)
d.rect(s, M, Inches(2.6), CW2, Inches(3.4), CARD)
d.bar(s, M + Inches(0.34), Inches(2.94), Inches(0.5), Inches(0.045), MUTED)
d.tb(s, "Without the intelligence", M + Inches(0.34), Inches(3.18),
     CW2 - Inches(0.68), Inches(0.4), 17, TEXT, bold=True)
d.bullets(s, ["The user selects settings by estimation.",
              "The machine executes without measurement.",
              "No result is retained.",
              "Each new coffee restarts the process."],
          M + Inches(0.34), Inches(3.78), CW2 - Inches(0.68), Inches(2.0), 14,
          MUTED, gap=10, marker_color=MUTED)
d.rect(s, rx, Inches(2.6), CW2, Inches(3.4), CARD_HI)
d.bar(s, rx + Inches(0.34), Inches(2.94), Inches(0.5), Inches(0.045), ACCENT)
d.tb(s, "With the intelligence", rx + Inches(0.34), Inches(3.18),
     CW2 - Inches(0.68), Inches(0.4), 17, TEXT, bold=True)
d.bullets(s, ['User: "18 grams in, light Ethiopian."',
              'System: "36 g in 22 seconds, reported sour. Reduce grind by two steps."',
              'System: "28 seconds, reported balanced. Store as a recipe?"',
              'User: "Make my usual." The recipe is executed.'],
          rx + Inches(0.34), Inches(3.78), CW2 - Inches(0.68), Inches(2.1), 14,
          TEXT, gap=10)
d.tb(s, "Convergence in 2 to 3 cycles in place of 5 to 10.",
     M, Inches(6.25), Inches(11), Inches(0.4), 15, ACCENT, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 08 UX
s = d.slide()
d.heading(s, "User-visible behaviour", "No application and no account.")
d.bullets(s, [
    "Nothing is installed, registered or configured. The intelligence is part of the machine.",
    "It identifies channeling from flow data and attributes the fault to preparation rather than to grind size.",
    "Target values shift toward the user's rated preferences over successive cycles.",
    "Settings are corrected as bean age changes the extraction rate.",
    "All processing is local. The machine does not require a network connection.",
], M, Inches(2.9), Inches(10.8), Inches(3.2), 17, gap=16)

# ---------------------------------------------------------------- 09 Adoption
s = d.slide()
d.heading(s, "Manufacturer requirements", "Cost, safety and certification.")
d.bullets(s, [
    ("Component cost.  ", "A bounded domain requires a small model, which runs on hardware within a premium appliance budget."),
    ("No recurring cost.  ", "Local processing means no per-unit cloud expense and no user account to support."),
    ("Bounded behaviour.  ", "The intelligence can only perform actions the manufacturer implemented and declared. No other action is reachable."),
    ("Certification.  ", "Behaviour can be enumerated and tested, which is a precondition for regulatory approval."),
    ("Brand.  ", "The feature ships under the manufacturer's name and raises the price of the machine."),
], M, Inches(2.9), Inches(10.9), Inches(3.3), 16.5, gap=17)

# ---------------------------------------------------------------- 10 Portability
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
d.tb(s, "Each additional domain reuses the engine and adds a knowledge base. "
        "The 850 million unit figure below depends on this.",
     M, Inches(6.16), Inches(11.2), Inches(0.4), 13.5, MUTED,
     align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 11 Market
s = d.slide()
d.heading(s, "Market", "Revenue is per unit shipped.")
y = Inches(2.7)
d.stat(s, M, y, CW3, Inches(2.55), "18M",
       "Home espresso machines sold worldwide, 2024",
       "Europe 6.5M, North America 5.1M, Asia-Pacific 2.8M.", ACCENT)
d.stat(s, M + CW3 + G3, y, CW3, Inches(2.55), "$3.4B",
       "Super-automatic espresso segment, 2025",
       "The premium subset. Estimates across firms range $3.4B to $5.2B.", ACCENT)
d.stat(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.55), "850M",
       "Major home appliances shipped per year",
       "The addressable scope once further domains are implemented.", TEAL)
d.rect(s, M, Inches(5.4), Inches(11.0), Inches(0.82), CARD_HI)
d.tb(s, "At a $12 royalty and 5% adoption, the espresso segment alone returns "
        "$3.0M per year.",
     M + Inches(0.4), Inches(5.64), Inches(10.2), Inches(0.45), 16, TEXT, bold=True)
d.tb(s, "Sources: Market Growth Reports, home espresso machine volumes 2024. "
        "Intel Market Research and Market Growth Reports, super-automatic "
        "segment 2025. Published estimates for these markets vary between firms.",
     M, Inches(6.36), Inches(11.2), Inches(0.4), 10.5, MUTED,
     align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 11 Revenue
s = d.slide()
d.heading(s, "Revenue model", "Four lines, one recurring.")
y = Inches(2.95)
for i, (t, b) in enumerate([
    ("Integration fee", "One-time payment covering the engineering to bring a manufacturer's first product to market."),
    ("Per-unit royalty", "A fee on each machine shipped, for the production life of the model."),
    ("Module margin", "Hardware margin on the compute module, for manufacturers who do not integrate the software directly."),
    ("Updates", "Annual subscription for revised models and additional domain knowledge."),
]):
    d.card(s, M + i * (CW4 + G4), y, CW4, Inches(2.8), t, b, ACCENT, 16, 12.5)
d.tb(s, "Appliance models remain in production for five to seven years, so a "
        "single integration produces royalty income across that period.",
     M, Inches(6.05), Inches(11.2), Inches(0.5), 15, MUTED,
     align=PP_ALIGN.CENTER, line=1.35)

# ---------------------------------------------------------------- 12 Economics
s = d.slide()
d.heading(s, "Unit economics", "Royalty is below the price\nincrease it supports.")
y = Inches(3.05)
d.card(s, M, y, CW2, Inches(2.5), "Manufacturer",
       "Adds a differentiating feature to a premium machine. The retail price "
       "increase exceeds the royalty. No cloud infrastructure, no account "
       "system and no associated support cost.", ACCENT)
d.card(s, M + CW2 + Inches(0.3), y, CW2, Inches(2.5), "StreetAI",
       "Software margin on the licence and royalty. Hardware margin on the "
       "module. A small number of customers, each on a multi-year contract.", TEAL)
d.tb(s, "Sales cycles in this sector run 18 to 36 months. Customer count is low "
        "and contract value is high.",
     M, Inches(6.0), Inches(11.2), Inches(0.5), 15, MUTED,
     align=PP_ALIGN.CENTER, line=1.35)

# ---------------------------------------------------------------- 13 Sequence
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

# ---------------------------------------------------------------- 14 Landscape
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

# ---------------------------------------------------------------- 15 Moat
s = d.slide()
d.heading(s, "Defensibility", "Four sources.")
d.bullets(s, [
    ("Integration standard.  ", "Once a manufacturer has implemented against the interface, replacement costs them a full product cycle."),
    ("Domain knowledge.  ", "Each domain encodes accumulated process expertise. It is independent of the hardware it runs on."),
    ("Field data.  ", "Deployed units generate measurements that improve subsequent models."),
    ("Certification mark.  ", "A consumer-visible mark creates demand from the manufacturer's own customers."),
], M, Inches(2.9), Inches(11.0), Inches(3.2), 16.5, gap=18)

# ---------------------------------------------------------------- 16 Status
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

# ---------------------------------------------------------------- 17 Team
s = d.slide()
d.heading(s, "Team", "[[Names and roles]]")
y = Inches(2.8)
d.card(s, M, y, CW3, Inches(2.5), "[[Founder name]]",
       "[[Role. Relevant experience in one sentence.]]", ACCENT)
d.card(s, M + CW3 + G3, y, CW3, Inches(2.5), "[[Name or open role]]",
       "[[Embedded systems and hardware.]]", MUTED)
d.card(s, M + 2 * (CW3 + G3), y, CW3, Inches(2.5), "[[Advisor or open role]]",
       "[[Appliance industry access.]]", MUTED)
d.tb(s, "[[Complete before sending.]]",
     M, Inches(5.6), Inches(11.2), Inches(0.4), 13, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 18 Funds
s = d.slide()
d.heading(s, "Use of funds", "$750,000 over 18 months.")
y = Inches(2.95)
for i, (pct, t, b) in enumerate([
    ("50%", "Engineering", "$375k. Two engineers for 18 months. Completes the espresso product and validates it on production machines with users."),
    ("15%", "Hardware", "$113k. Reference compute module on qualified silicon, prototype runs, and radio and safety certification."),
    ("20%", "Business development", "$150k. Manufacturer engagement, trade attendance, evaluation units and the first integration."),
    ("15%", "Operations", "$112k. Legal, intellectual property filing and administration."),
]):
    d.rect(s, M + i * (CW4 + G4), y, CW4, Inches(2.85))
    d.tb(s, pct, M + i * (CW4 + G4) + Inches(0.3), y + Inches(0.32),
         CW4 - Inches(0.6), Inches(0.5), 26, ACCENT, bold=True)
    d.tb(s, t, M + i * (CW4 + G4) + Inches(0.3), y + Inches(0.92),
         CW4 - Inches(0.6), Inches(0.4), 15, TEXT, bold=True)
    d.tb(s, b, M + i * (CW4 + G4) + Inches(0.3), y + Inches(1.34),
         CW4 - Inches(0.6), Inches(1.3), 12.5, MUTED, line=1.35)
d.tb(s, "Appliance sales cycles run 18 to 36 months, so the round is sized to "
        "reach signature rather than revenue.",
     M, Inches(6.05), Inches(11.2), Inches(0.4), 14, MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- 19 Ask
s = d.slide(number=False)
d.bar(s, M, Inches(1.9), Inches(1.3), Inches(0.06), ACCENT)
d.tb(s, "Funding requirement", M, Inches(2.3), CW, Inches(0.4), 12, MUTED,
     bold=True, caps=True)
d.tb(s, "$750,000 to reach the first\nmanufacturer agreement.", M,
     Inches(2.75), Inches(11), Inches(1.7), 38, TEXT, bold=True, line=1.1)
d.bullets(s, [
    "Validate the espresso product on production machines with users.",
    "Publish the integration standard and secure implementations against it.",
    "Produce the reference compute module.",
    "Complete the first manufacturer agreement.",
], M, Inches(4.5), Inches(9.5), Inches(2.0), 16.5, gap=13)

d.save(r"C:\T-Works\domain-intelligence\StreetAI-ADI-Investor-Deck.pptx")
