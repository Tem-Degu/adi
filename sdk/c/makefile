# libadip build.
#
# The warning set is deliberately hostile. This code goes into appliances, and
# a warning here is cheaper than a field failure.

# `CC ?= gcc` does not work here: make predefines CC as `cc`, so `?=` sees it
# already set and the default never applies. Toolchains that ship only `gcc`
# then fail with "cannot find cc". Overriding on the command line still wins,
# because that origin is `command line`, not `default`.
ifeq ($(origin CC),default)
CC = gcc
endif

CSTD    ?= -std=c99

WARN = -Wall -Wextra -pedantic \
       -Wshadow -Wcast-align -Wcast-qual -Wconversion -Wsign-conversion \
       -Wstrict-prototypes -Wmissing-prototypes -Wredundant-decls \
       -Wundef -Wpointer-arith -Wwrite-strings -Wswitch-enum \
       -Wdouble-promotion -Wformat=2

CFLAGS ?= $(CSTD) $(WARN) -Os -Iinclude -Igenerated

# CORE is everything that touches no platform. Only adip_runtime.c calls the
# port layer, so CORE links and tests with no platform at all, which is what
# lets a manufacturer run the conformance checks before any board exists.
CORE = src/adip_num.c \
       src/adip_tables.c \
       src/adip_cbor.c \
       src/adip_frame.c \
       src/adip_decode.c \
       src/adip_validate.c \
       src/adip_state.c

RUNTIME = $(CORE) src/adip_runtime.c

SRC = $(RUNTIME)

GEN     = generated/adip_generated_neutral.c generated/adip_generated_ex900.c
NEUTRAL = generated/adip_generated_neutral.c

T1 = test/test_m1.c
T2 = test/test_m2.c
T3 = test/test_m3.c
T4 = test/test_m4.c
T5 = test/test_m5.c
T6 = test/test_signals.c
T7 = test/test_setpoints.c

BUILD = build
OBJ   = $(patsubst src/%.c,$(BUILD)/%.o,$(RUNTIME))

# A target whose name matches a directory needs declaring here, or make finds
# the directory, calls it up to date and does nothing. `example` was declared
# for that reason; `arm` and `dehydrator` collide with arm/ and dehydrator/ the
# same way and were not.
.PHONY: all test clean size regen example arm arm-test arm-size dehydrator dehydrator-run shower shower-run

TESTS = $(BUILD)/test_m1 $(BUILD)/test_m2 $(BUILD)/test_m3 $(BUILD)/test_m4 \
        $(BUILD)/test_m5 $(BUILD)/test_signals $(BUILD)/test_setpoints

all: $(TESTS)

$(BUILD):
	mkdir -p $(BUILD)

$(BUILD)/test_m1: $(CORE) $(GEN) $(T1) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(CORE) $(GEN) $(T1)

$(BUILD)/test_m2: $(CORE) $(NEUTRAL) $(T2) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(CORE) $(NEUTRAL) $(T2)

$(BUILD)/test_m3: $(SRC) $(NEUTRAL) $(T3) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(NEUTRAL) $(T3)

$(BUILD)/test_m4: $(SRC) $(NEUTRAL) $(T4) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(NEUTRAL) $(T4)

$(BUILD)/test_m5: $(SRC) $(NEUTRAL) $(T5) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(NEUTRAL) $(T5)

$(BUILD)/test_signals: $(SRC) $(NEUTRAL) $(T6) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(NEUTRAL) $(T6)

$(BUILD)/test_setpoints: $(SRC) $(NEUTRAL) $(T7) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(NEUTRAL) $(T7)

EX900 = generated/adip_generated_ex900.c
EXAMPLE = example/main.c

# A complete appliance, for an engineer to copy and for the engine to talk to.
# ADIP_ENABLE_FLOAT is for the shot simulation only. A device build never
# defines it, and the protocol facing code in the example does not use it.
$(BUILD)/example: $(SRC) $(EX900) $(EXAMPLE) | $(BUILD)
	$(CC) $(CFLAGS) -DADIP_ENABLE_FLOAT -o $@ $(SRC) $(EX900) $(EXAMPLE)

example: $(BUILD)/example

test: $(TESTS)
	./$(BUILD)/test_m1
	./$(BUILD)/test_m2
	./$(BUILD)/test_m3
	./$(BUILD)/test_m4
	./$(BUILD)/test_m5
	./$(BUILD)/test_signals
	./$(BUILD)/test_setpoints

# Regenerate the tables from the declarations. Run after editing either.
regen:
	cd .. && cargo run --quiet -p adi-cli -- generate-c \
	    examples/declarations/neutral-machine.yaml --out c/generated --name neutral
	cd .. && cargo run --quiet -p adi-cli -- generate-c \
	    examples/declarations/ex900.yaml --out c/generated --name ex900
	cd .. && cargo run --quiet -p adi-cli -- generate-c \
	    examples/declarations/shower.yaml --out c/generated --name sx2

# Footprint of the library alone, without the test harness. DESIGN.md section 8
# targets under 12 KB of flash for the whole runtime.
$(BUILD)/%.o: src/%.c | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

size: $(OBJ) | $(BUILD)
	$(CC) $(CFLAGS) -c $(NEUTRAL) -o $(BUILD)/gen_neutral.o
	size $(OBJ) $(BUILD)/gen_neutral.o

clean:
	rm -rf $(BUILD)

# ---------------------------------------------------------------- ARM target
#
# Runs the same test sources on an emulated Cortex-M3, which is the kind of
# core an appliance actually uses. The host tests prove the logic; these prove
# it survives a 32-bit core, where the library's 64-bit fixed-point arithmetic
# is synthesised from smaller operations rather than performed directly.
#
#   mingw32-make arm-size    footprint for Cortex-M4
#   mingw32-make arm-test    run every suite under QEMU
#   mingw32-make dehydrator  a second machine as firmware, UART over QEMU
#
# Needs arm-none-eabi-gcc and qemu-system-arm. Both are free; neither needs a
# board. Override the paths if they are not on PATH.

ARMCC   ?= arm-none-eabi-gcc
ARMSIZE ?= arm-none-eabi-size
QEMU    ?= qemu-system-arm

ARMBUILD = $(BUILD)/arm
ARMCPU  ?= cortex-m3
ARMFLAGS = -std=c99 -Os -mcpu=$(ARMCPU) -mthumb -Iinclude -Igenerated $(WARN)
ARMLD    = arm/lm3s6965.ld
ARMLINK  = --specs=nano.specs -nostartfiles -T $(ARMLD)
ARMLIBS  = -lc -lnosys

ARMTESTS = $(ARMBUILD)/test_m1.elf $(ARMBUILD)/test_m2.elf $(ARMBUILD)/test_m3.elf \
           $(ARMBUILD)/test_m4.elf $(ARMBUILD)/test_m5.elf \
           $(ARMBUILD)/test_signals.elf $(ARMBUILD)/test_setpoints.elf

$(ARMBUILD):
	mkdir -p $(ARMBUILD)

# M1 and M2 need no runtime; the rest do, and define the port functions
# themselves.
$(ARMBUILD)/test_m1.elf: arm/startup.c $(ARMLD) $(CORE) $(GEN) $(T1) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(CORE) $(GEN) $(T1) -o $@ $(ARMLIBS)

$(ARMBUILD)/test_m2.elf: arm/startup.c $(ARMLD) $(CORE) $(NEUTRAL) $(T2) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(CORE) $(NEUTRAL) $(T2) -o $@ $(ARMLIBS)

$(ARMBUILD)/test_m3.elf: arm/startup.c $(ARMLD) $(SRC) $(NEUTRAL) $(T3) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(NEUTRAL) $(T3) -o $@ $(ARMLIBS)

$(ARMBUILD)/test_m4.elf: arm/startup.c $(ARMLD) $(SRC) $(NEUTRAL) $(T4) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(NEUTRAL) $(T4) -o $@ $(ARMLIBS)

$(ARMBUILD)/test_m5.elf: arm/startup.c $(ARMLD) $(SRC) $(NEUTRAL) $(T5) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(NEUTRAL) $(T5) -o $@ $(ARMLIBS)

$(ARMBUILD)/test_signals.elf: arm/startup.c $(ARMLD) $(SRC) $(NEUTRAL) $(T6) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(NEUTRAL) $(T6) -o $@ $(ARMLIBS)

$(ARMBUILD)/test_setpoints.elf: arm/startup.c $(ARMLD) $(SRC) $(NEUTRAL) $(T7) | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(NEUTRAL) $(T7) -o $@ $(ARMLIBS)

arm: $(ARMTESTS)

# ----------------------------------------------- a second machine, as firmware
#
# A food dehydrator on the base profile, built for the same emulated core. It
# exists to answer one question: whether a manufacturer can describe a machine
# nobody wrote a profile for and have the engine reason about it. See
# examples/declarations/dehydrator.yaml and HOW-TO-PORT-TO-A-BOARD.md.
#
#   make dehydrator      build the firmware
#   make dehydrator-run  run it under QEMU with UART0 on a pty, which is what
#                        `adi run --serial <pty>` connects to
#
# Unlike the test suites this is not semihosted: the link is UART0, so the
# bytes cross a real peripheral and the engine's own framing.

DH20 = generated/adip_generated_dh20.c

$(ARMBUILD)/dehydrator.elf: arm/startup.c $(ARMLD) $(SRC) $(DH20) dehydrator/main.c | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(DH20) dehydrator/main.c -o $@ $(ARMLIBS)

dehydrator: $(ARMBUILD)/dehydrator.elf

# -serial pty makes QEMU print the device it allocated. Hand that to
# `adi run --serial`.
dehydrator-run: $(ARMBUILD)/dehydrator.elf
	$(QEMU) -M lm3s6965evb -nographic -serial pty -kernel $(ARMBUILD)/dehydrator.elf

# ------------------------------------------------ a third machine, as firmware
#
# A thermostatic shower mixer, on the base profile, for the same emulated
# core. Two mechanisms rather than one, so a spoken request has to reach the
# right one. See examples/declarations/shower.yaml.
#
#   make shower      build the firmware
#   make shower-run  run it under QEMU with UART0 on a pty

SX2 = generated/adip_generated_sx2.c

$(ARMBUILD)/shower.elf: arm/startup.c $(ARMLD) $(SRC) $(SX2) shower/main.c | $(ARMBUILD)
	$(ARMCC) $(ARMFLAGS) $(ARMLINK) arm/startup.c $(SRC) $(SX2) shower/main.c -o $@ $(ARMLIBS)

shower: $(ARMBUILD)/shower.elf

shower-run: $(ARMBUILD)/shower.elf
	$(QEMU) -M lm3s6965evb -nographic -serial pty -kernel $(ARMBUILD)/shower.elf

# Every ARM rule lists its sources. They were order-only before, with nothing
# but the build directory as a prerequisite, so once a .elf existed make
# considered it current and never rebuilt it. `make arm-test` then ran whatever
# was compiled first and reported it as a pass, which is worse than failing:
# the suite that exists to check this code on a 32-bit core was checking an
# older copy of it.

# QEMU reports the suite's own exit status, so a failure fails the make.
arm-test: $(ARMTESTS)
	@for t in $(ARMTESTS); do \
	  echo "--- $$t"; \
	  $(QEMU) -M lm3s6965evb -nographic -semihosting -kernel $$t || exit 1; \
	done

# The runtime alone, without any machine's generated tables, which is the
# figure comparable to the host build.
arm-size: | $(ARMBUILD)
	@for f in $(RUNTIME); do \
	  $(ARMCC) $(ARMFLAGS) -ffreestanding -c $$f -o $(ARMBUILD)/`basename $$f .c`.o; \
	done
	$(ARMSIZE) -t $(ARMBUILD)/adip_*.o | tail -1
