.DEFAULT_GOAL := help

CARGO ?= cargo

.PHONY: help format format-check check test build run run-gui run-release clippy boundary clean

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
	$(CARGO) run -- $(ARGS)

run-gui: ## Run the Iced desktop GUI
	$(CARGO) run --bin denon-avr-remote-gui

run-release: ## Run the CLI with release optimizations (use ARGS="...")
	$(CARGO) run --release -- $(ARGS)

clippy: ## Run Clippy with warnings treated as errors
	$(CARGO) clippy --all-targets -- -D warnings

clean: ## Remove Cargo build artifacts
	$(CARGO) clean
