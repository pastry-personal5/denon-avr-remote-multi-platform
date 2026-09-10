.DEFAULT_GOAL := help

CARGO ?= cargo

.PHONY: help format format-check check test build run run-gui run-release run-diagnostics run-diagnostics-http package-macos capture-visual-baselines visual-baselines clippy boundary clean

help: ## Show available developer commands
	@echo "Available targets:"
	@grep -E '^[a-zA-Z0-9_-]+:.*## ' $(MAKEFILE_LIST) | sed 's/:.*## /\t/'

format: ## Format Rust source files
	$(CARGO) fmt --all

format-check: ## Verify Rust formatting without changing files
	$(CARGO) fmt --all -- --check

check: format-check boundary ## Run formatting, boundary, compilation, and unit-test checks
	$(CARGO) check --all-targets
	$(CARGO) test --all-targets

boundary: ## Check canonical layer dependency boundaries
	tools/check-boundaries.sh

test: ## Run the test suite
	$(CARGO) test --all-targets

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
