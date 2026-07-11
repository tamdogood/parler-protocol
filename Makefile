# Parler Protocol Developer Makefile
#
# Shortcuts for building, running, and managing Parler Protocol agents and hubs.
# Note: Ensure Rust (cargo) and Node.js are installed.

# Automatically locate cargo, falling back to ~/.cargo/bin/cargo if not in PATH
CARGO := $(shell which cargo 2>/dev/null || echo $(HOME)/.cargo/bin/cargo)

# CLI Command Runner & Shared Configuration
PARLER_RUN := $(CARGO) run -p parler-bin --
HUB_ADDR   := 127.0.0.1:7070
HUB_URL    := parler://$(HUB_ADDR)

# Default variables for agent targets (supports both upper and lowercase)
AGENT_NAME   ?= $(or $(name),$(NAME),atlas)
AGENT_STATE  ?= $(or $(state),$(STATE),on)
AGENT_ROLE   ?= $(or $(role),$(ROLE),assistant)
AGENT_DESC   ?= $(or $(desc),$(DESC),A helper agent on the mesh)
AGENT_VIS    ?= $(or $(visibility),$(VISIBILITY),public)
AGENT_TAGS   ?= $(or $(tags),$(TAGS),)
AGENT_SKILLS ?= $(or $(skills),$(SKILLS),)

.PHONY: help build run-demo run-hub stop discover agent-active add-agent clean test dev \
        ci selftest audit smoke coverage

# Default target showing help
help:
	@echo "========================================================================="
	@echo "🛰️  PARLER DEVELOPER ACTIONS"
	@echo "========================================================================="
	@echo "Building:"
	@echo "  make build                     Build the Rust workspace"
	@echo "  make test                      Run the Rust unit tests (excluding external deps)"
	@echo "Quality / CI (run before you push — mirrors GitHub CI):"
	@echo "  make ci                        Run the WHOLE local pipeline (build·clippy·test·audit)"
	@echo "  make selftest                  Test the test system (scripts, lib.sh, config sanity)"
	@echo "  make audit                     Supply-chain scan via cargo-deny (deny.toml)"
	@echo "  make smoke                     Boot the real hub binary & probe its HTTP surface"
	@echo "  make coverage                  Test coverage report (needs cargo-llvm-cov)"
	@echo "Running:"
	@echo "  make dev                       Start the demo hub seeded with mock agents"
	@echo "  make run-demo                  Start the demo hub seeded with mock agents"
	@echo "  make run-hub                   Start a standalone public hub on port 7070"
	@echo "Monitoring & Management:"
	@echo "  make discover                  List all registered public agents"
	@echo "  make agent-active [name=<name>] [state=on|off]"
	@echo "                                 Set agent presence on (working) or off (offline)"
	@echo "                                 (defaults: name=atlas, state=on)"
	@echo "  make add-agent [name=<name>] [role=<role>] [desc=<desc>]"
	@echo "                 [visibility=public|private] [tags=t1,t2] [skills=s1,s2]"
	@echo "                                 Create and register a new agent on the hub"
	@echo "Clean:"
	@echo "  make clean                     Remove build artifacts and SQLite demo database"
	@echo "========================================================================="
	@echo "📝 EXAMPLES FOR ALL COMMANDS:"
	@echo "  * Build application and assets:             make build"
	@echo "  * Run unit tests:                           make test"
	@echo "  * Start seeded hub:                         make run-demo"
	@echo "  * Start blank hub only:                     make run-hub"
	@echo "  * List all public agents:                   make discover"
	@echo "  * Toggle agent presence (online):           make agent-active name=forge state=on"
	@echo "  * Toggle agent presence (offline):          make agent-active name=forge state=off"
	@echo "  * Add a new agent:                          make add-agent name=compiler role=coder desc='Compiles Rust' tags=rust skills=compile"
	@echo "  * Reset and clean dev files:                make clean"
	@echo "========================================================================="

# Build everything
build:
	@echo "→ Building Rust workspace using: $(CARGO)"
	$(CARGO) build --workspace

# Run library unit tests (skips integration tests requiring external nats-server dependency)
test:
	@echo "→ Running workspace unit tests..."
	$(CARGO) test --workspace --lib

# --- Quality / CI -----------------------------------------------------------------------------------
# The full local pipeline — identical gates to .github/workflows/ci.yml. Run this before pushing.
ci:
	@./scripts/ci/all.sh

# Test the test system: every CI script's syntax + the lib.sh step runner + workflow/deny.toml sanity.
selftest:
	@./scripts/ci/selftest.sh

# Supply-chain gate. Installs cargo-deny on demand if it's missing, then runs the same audit as CI.
audit:
	@command -v cargo-deny >/dev/null 2>&1 || { echo "→ installing cargo-deny..."; $(CARGO) install cargo-deny --locked; }
	@./scripts/ci/audit.sh

# Boot the freshly-built hub binary and assert its HTTP contract (the deploy-time smoke, locally).
smoke:
	@./scripts/ci/smoke.sh --boot

# HTML coverage report at target/llvm-cov/html/index.html. Needs: cargo install cargo-llvm-cov
coverage:
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { echo "Install it first: cargo install cargo-llvm-cov"; exit 1; }
	$(CARGO) llvm-cov --workspace --html
	@echo "→ Open target/llvm-cov/html/index.html"

# Start the demo hub seeded with mock agents (the website now lives in the parler-web repo)
dev:
	@echo "→ Starting seeded demo..."
	PATH="$$HOME/.cargo/bin:$$PATH" ./scripts/seed-demo.sh

# Start a public seed hub (includes 7 mock agents with automatic refresher)
run-demo:
	@echo "→ Starting seeded demo..."
	PATH="$$HOME/.cargo/bin:$$PATH" ./scripts/seed-demo.sh

# Start a clean public hub without seeding
run-hub:
	@echo "→ Starting standalone Parler Protocol Hub..."
	$(PARLER_RUN) hub --public --name "Parler Protocol Hub" --db ./hub.sqlite --addr $(HUB_ADDR)

# List all public agents registered in the hub (requires demo workspace configuration)
discover:
	@if [ -d ".demo/atlas" ]; then \
		PARLER_HOME=.demo/atlas $(PARLER_RUN) discover --public; \
	else \
		echo "Error: No demo identity found. Run 'make run-demo' first."; \
		exit 1; \
	fi

# Set agent presence to working (state=on) or offline (state=off)
agent-active:
	@if [ "$(AGENT_STATE)" = "on" ]; then \
		STATUS="working"; \
		ARGS="--activity Active"; \
	elif [ "$(AGENT_STATE)" = "off" ]; then \
		STATUS="offline"; \
		ARGS=""; \
	else \
		echo "Error: state/STATE must be 'on' or 'off'"; \
		exit 1; \
	fi; \
	if [ -d ".demo/$(AGENT_NAME)" ]; then \
		echo "→ Setting agent '$(AGENT_NAME)' status to '$$STATUS'..."; \
		PARLER_HOME=.demo/$(AGENT_NAME) $(PARLER_RUN) presence $$STATUS $$ARGS; \
	else \
		echo "Error: No demo identity found for '$(AGENT_NAME)' at .demo/$(AGENT_NAME)."; \
		exit 1; \
	fi

# Initialize, register, and set presence for a new custom agent
add-agent:
	@echo "→ Initializing identity for agent '$(AGENT_NAME)'..."
	@PARLER_HOME=.demo/$(AGENT_NAME) $(PARLER_RUN) init --hub $(HUB_URL) --name $(AGENT_NAME) --role $(AGENT_ROLE) --force
	@echo "→ Registering card for agent '$(AGENT_NAME)'..."
	@VIS_FLAG=""; \
	if [ "$(AGENT_VIS)" = "public" ]; then VIS_FLAG="--public"; fi; \
	TAGS_ARGS=""; \
	if [ -n "$(AGENT_TAGS)" ]; then \
		for t in $$(echo "$(AGENT_TAGS)" | tr ',' ' '); do \
			TAGS_ARGS="$$TAGS_ARGS --tag $$t"; \
		done; \
	fi; \
	SKILLS_ARGS=""; \
	if [ -n "$(AGENT_SKILLS)" ]; then \
		for s in $$(echo "$(AGENT_SKILLS)" | tr ',' ' '); do \
			SKILLS_ARGS="$$SKILLS_ARGS --skill $$s"; \
		done; \
	fi; \
	PARLER_HOME=.demo/$(AGENT_NAME) $(PARLER_RUN) register $$VIS_FLAG --describe "$(AGENT_DESC)" $$TAGS_ARGS $$SKILLS_ARGS
	@echo "→ Setting initial presence to online..."
	@PARLER_HOME=.demo/$(AGENT_NAME) $(PARLER_RUN) presence idle
	@echo "✓ Agent '$(AGENT_NAME)' successfully added and registered!"

# Clean database files and build outputs
clean:
	@echo "→ Cleaning database files..."
	rm -rf .demo/ hub.sqlite .demo_hub.log
	@echo "→ Cleaning cargo targets..."
	$(CARGO) clean
