# Parler Developer Makefile
#
# Shortcuts for building, running, and managing Parler agents and hubs.
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

.PHONY: help build run-demo run-hub run-web stop discover agent-active add-agent clean test dev

# Default target showing help
help:
	@echo "========================================================================="
	@echo "🛰️  PARLER DEVELOPER ACTIONS"
	@echo "========================================================================="
	@echo "Building:"
	@echo "  make build                     Build Rust workspace & install npm dependencies"
	@echo "  make test                      Run the Rust unit tests (excluding external deps)"
	@echo "Running:"
	@echo "  make dev                       Start BOTH the demo hub and Web UI together"
	@echo "  make run-demo                  Start the demo hub seeded with mock agents"
	@echo "  make run-hub                   Start a standalone public hub on port 7070"
	@echo "  make run-web                   Start the Next.js directory website"
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
	@echo "  * Start Dev Mesh (Hub + Web UI):            make dev"
	@echo "  * Start seeded hub only:                    make run-demo"
	@echo "  * Start blank hub only:                     make run-hub"
	@echo "  * Start Web UI only:                        make run-web"
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
	@echo "→ Installing frontend dependencies..."
	cd web && npm install

# Run library unit tests (skips integration tests requiring external nats-server dependency)
test:
	@echo "→ Running workspace unit tests..."
	$(CARGO) test --workspace --lib

# Start BOTH the demo hub and Web UI concurrently
dev:
	@echo "→ Starting seeded demo hub in background..."
	@PATH="$$HOME/.cargo/bin:$$PATH" ./scripts/seed-demo.sh > .demo_hub.log 2>&1 & HUB_PID=$$!; \
	echo "→ Waiting for hub to initialize..." && sleep 4; \
	echo "→ Starting Web UI on http://localhost:3000..."; \
	cd web && NEXT_PUBLIC_HUB_API=http://$(HUB_ADDR) npm run dev; \
	echo "→ Shutting down background hub (PID $$HUB_PID)..."; \
	kill $$HUB_PID 2>/dev/null || true

# Start a public seed hub (includes 7 mock agents with automatic refresher)
run-demo:
	@echo "→ Starting seeded demo..."
	PATH="$$HOME/.cargo/bin:$$PATH" ./scripts/seed-demo.sh

# Start a clean public hub without seeding
run-hub:
	@echo "→ Starting standalone Parler Hub..."
	$(PARLER_RUN) hub --public --name "Parler Hub" --db ./hub.sqlite --addr $(HUB_ADDR)

# Start the web directory site
run-web:
	@echo "→ Starting Web UI on http://localhost:3000..."
	cd web && NEXT_PUBLIC_HUB_API=http://$(HUB_ADDR) npm run dev

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
