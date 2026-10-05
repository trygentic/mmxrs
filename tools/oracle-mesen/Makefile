# Builds Mesen2's emulation core (C++ only, no .NET UI, no SDL/Cocoa front-end)
# as a static library inside Mesen2's own ignored build/ dir, and links the
# headless oracle harness against it. See README.md.

MESEN ?= /Users/ritz/dev/Mesen2
BUILD ?= $(MESEN)/build/oracle
OUT ?= $(BUILD)/mesen-oracle

CXX := clang++
CC := clang
OPT ?= -O2
MESENINC := -I$(MESEN) -I$(MESEN)/Core -I$(MESEN)/Utilities
CXXFLAGS := -fPIC -w --std=c++17 $(OPT) $(MESENINC)
CFLAGS := -fPIC -w $(OPT)

SRC := $(shell cd $(MESEN) && find Core Utilities Lua SevenZip \( -name '*.cpp' -o -name '*.c' \))
OBJ := $(addprefix $(BUILD)/obj/,$(addsuffix .o,$(basename $(SRC))))
LIB := $(BUILD)/libmesencore.a

all: $(OUT)

$(BUILD)/obj/%.o: $(MESEN)/%.cpp
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

$(BUILD)/obj/%.o: $(MESEN)/%.c
	@mkdir -p $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

# Fixes applied to a copy of one Mesen source file (the clone stays untouched):
# RunDma counts a channel's bytes in a uint8_t, so a DMA of more than 255 bytes
# adds (bytes mod 256) * 8 clocks to the counter that SyncEndDma aligns to the
# CPU clock: the end padding comes out wrong (2-4 clocks) for big DMAs.
PATCHED := $(BUILD)/patched/Core/SNES/SnesDmaController.cpp
$(PATCHED): $(MESEN)/Core/SNES/SnesDmaController.cpp
	@mkdir -p $(dir $@)
	sed '81s/uint8_t i = 0;/uint32_t i = 0; \/\/ mesen-oracle patch: was uint8_t/' $< > $@
	grep -q 'mesen-oracle patch' $@

$(BUILD)/obj/Core/SNES/SnesDmaController.o: $(PATCHED)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -I$(MESEN)/Core/SNES -c $< -o $@

# --bsnes-apu: the SPC clock ratio of bsnes v0.87 (the movies' emulator):
# 24,607,104 Hz SPC master per 21,477,272 Hz CPU master. Mesen's own setting is
# an integer sample rate (32000 + adjustment), which cannot express it.
PATCHED_SPC := $(BUILD)/patched/Core/SNES/Spc.cpp
$(PATCHED_SPC): $(MESEN)/Core/SNES/Spc.cpp
	@mkdir -p $(dir $@)
	sed -e '126s/(double)(_spcSampleRate \* 64)/(mesenOracleSpcRate > 0 ? mesenOracleSpcRate : (double)(_spcSampleRate * 64))/' $< > $@.tmp
	(echo 'double mesenOracleSpcRate = 0; // mesen-oracle patch'; cat $@.tmp) > $@ && rm $@.tmp
	grep -q 'mesenOracleSpcRate > 0' $@

$(BUILD)/obj/Core/SNES/Spc.o: $(PATCHED_SPC)
	@mkdir -p $(dir $@)
	$(CXX) $(CXXFLAGS) -I$(MESEN)/Core/SNES -c $< -o $@

$(LIB): $(OBJ)
	rm -f $@
	ar rcs $@ $(OBJ)

$(OUT): harness.cpp $(LIB)
	$(CXX) $(CXXFLAGS) -o $@ harness.cpp $(LIB) -lz -pthread

clean:
	rm -rf $(BUILD)

.PHONY: all clean
