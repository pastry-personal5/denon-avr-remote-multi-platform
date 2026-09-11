.DEFAULT_GOAL := help

CARGO ?= cargo

.PHONY: help format format-check check test build run run-gui run-release run-diagnostics run-diagnostics-http package-macos capture-visual-baselines visual-baselines clippy boundary phase5-ledger clean test-core-smoke test-core-scenarios test-core-properties test-trace-replay test-desktop-model test-live-x3800h test-live-x3800h-controls

help: ## Show available developer commands
	@echo "Available targets:"
	@grep -E '^[a-zA-Z0-9_-]+:.*## ' $(MAKEFILE_LIST) | sed 's/:.*## /\t/'

format: ## Format Rust source files
	$(CARGO) fmt --all

format-check: ## Verify Rust formatting without changing files
	$(CARGO) fmt --all -- --check

check: format-check boundary phase5-ledger test-core-smoke test-core-scenarios test-core-properties test-trace-replay test-desktop-model ## Run formatting, boundaries, Phase 5 deterministic checks, compilation, and tests
	$(CARGO) check --all-targets
	$(CARGO) test --all-targets

boundary: ## Check canonical layer dependency boundaries
	tools/check-boundaries.sh

phase5-ledger: ## Verify Phase 5 requirement IDs and executable fixtures
	tools/check-phase5-ledger.sh

test: ## Run the test suite
	$(CARGO) test --all-targets

test-core-smoke: ## Run delivery-independent receiver-core smoke tests
	$(CARGO) test -p denon-avr-infrastructure --test core_smoke

test-core-scenarios: ## Run deterministic receiver-core scenarios
	$(CARGO) test -p denon-avr-domain -p denon-avr-protocol -p denon-avr-application -p denon-avr-infrastructure

test-core-properties: ## Run generated receiver-state properties
	$(CARGO) test -p denon-avr-domain receiver_state
	$(CARGO) test -p denon-avr-infrastructure --test receiver_properties

test-trace-replay: ## Replay the reviewed X3800H raw-frame corpus
	$(CARGO) test -p denon-avr-infrastructure --test x3800h_trace_replay

test-desktop-model: ## Run headless desktop projection tests
	$(CARGO) test -p denon-avr-gui-lib --lib

test-live-x3800h: ## Run opt-in read-only validation against a physical AVC-X3800H
	@test -n "$(DENON_X3800H_HOST)" || (echo "set DENON_X3800H_HOST to run live validation" >&2; exit 2)
	$(CARGO) test -p denon-avr-infrastructure --test live_x3800h -- --ignored

test-live-x3800h-controls: ## Run explicitly armed, state-restoring live validation
	@test "$${ALLOW_RECEIVER_WRITES}" = "1" || (echo "set ALLOW_RECEIVER_WRITES=1 to arm receiver writes" >&2; exit 2)
	@test -n "$(DENON_X3800H_HOST)" || (echo "set DENON_X3800H_HOST to run live validation" >&2; exit 2)
	@test -n "$${DENON_X3800H_SAFE_VOLUME_HALF_STEPS}" || (echo "set DENON_X3800H_SAFE_VOLUME_HALF_STEPS before live validation" >&2; exit 2)
	$(CARGO) test -p denon-avr-infrastructure --test live_x3800h_controls -- --ignored

build: ## Build the project
	$(CARGO) build

run: ## Run the CLI in debug mode (use ARGS="...")
	$(CARGO) run -p denon-avr-cli --bin denon-avr-remote -- $(ARGS)

run-gui: ## Run the Iced desktop GUI
	$(CARGO) run -p denon-avr-desktop --bin denon-avr-remote-gui

run-diagnostics: ## Run the diagnostics probe (use ARGS="...")
	$(CARGO) run -p denon-avr-diagnostics --bin x3800h-context-probe -- $(ARGS)

run-diagnostics-http: ## Run the HTTP diagnostics probe (use ARGS="...")
	$(CARGO) run -p denon-avr-diagnostics --bin x3800h-http-info-probe -- $(ARGS)

visual-baselines: ## Compare native GUI captures (use PLATFORM=macos|windows|linux CAPTURES=DIR)
	tools/check-visual-baselines.sh "$(PLATFORM)" "$(CAPTURES)"

capture-visual-baselines: ## Capture every deterministic GUI baseline (use CAPTURES=DIR)
	tools/capture-visual-baselines.sh "$(CAPTURES)"

run-release: ## Run the CLI with release optimizations (use ARGS="...")
	$(CARGO) run --release -p denon-avr-cli --bin denon-avr-remote -- $(ARGS)

package-macos: ## Build the unsigned Apple Silicon macOS app and DMG
	tools/package-macos.sh

clippy: ## Run Clippy with warnings treated as errors
	$(CARGO) clippy --all-targets -- -D warnings

clean: ## Remove Cargo build artifacts
	$(CARGO) clean
