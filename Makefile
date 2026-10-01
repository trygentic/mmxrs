# mmxrs: build and launch. ROMs are never committed; point MMX_ROM (and
# optionally MMX_X3_ROM) at your own dumps, or put them under roms/ (gitignored).
#
#   make            build release binaries and open the launcher
#   make run        build and start the game directly (ARGS="--coop-local ...")
#   make test       workspace tests (ROM-gated tests skip without MMX_ROM)
#   make check      clippy + wasm checks + parity verify
#
# The launcher spawns the game binary sitting next to it, so both are built.

ROM     ?= $(or $(MMX_ROM),roms/Mega Man X (USA) (Rev 1).sfc)
X3_ROM  ?= $(or $(MMX_X3_ROM),roms/Mega Man X3 (USA).sfc)
ARGS    ?=
BIN      = target/release/mmxrs
LAUNCHER = target/release/mmxrs-launcher

.PHONY: all build launcher run coop zero test check help

all: launcher

build:
	cargo build --release -p mmx-native -p mmx-launcher

launcher: build
	./$(LAUNCHER)

# Direct launch. X3 is passed only when the file exists (paths may contain spaces).
HAVE_X3 := $(shell test -f "$(X3_ROM)" && echo yes)
run: build
	./$(BIN) --rom "$(ROM)" $(if $(HAVE_X3),--x3-rom "$(X3_ROM)") $(ARGS)

coop: ARGS += --coop-local
coop: run

zero: ARGS += --zero
zero: run

test:
	cargo test --workspace

check:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo check -p mmx-core -p mmx-web --target wasm32-unknown-unknown
	MMX_ROM="$(ROM)" cargo run --release -q -p xtask -- verify --frames 2400 --coop-off --tap 200,400,600,800,1000 --hold 0100

help:
	@sed -n '2,10p' Makefile
