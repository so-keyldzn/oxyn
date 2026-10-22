# The repository's quality gate.
#
# `make qualite` is the ONLY entry point: the Stop hook reminds of it, the
# definition of "done" rests on it. A check added anywhere else is an optional
# check.
#
# CI (.github/workflows/qualite.yml) calls its pieces in parallel jobs —
# `front-controles`, `front-tests`, `front-build`, `rust` — so that Rust does
# not wait behind the stories. It adds nothing: `make socle` refuses a workflow
# that would forget a target reached by `qualite`. On a pull request, it skips
# the jobs whose area is not touched; on `main`, everything runs (ADR-0045).
#
# `make verif-rapide` is for day-to-day work: it only checks what changed since
# `origin/main`. It is not conclusive, `qualite` is.
#
# See .claude/rules/manifestes.md and .claude/checklists/fin-de-tache.md.

.PHONY: qualite verif-rapide format lint test test-serveurs doc deny todo versions hooks socle aide front front-controles front-tests front-build rust package-contracts desktop desktop-dev licences-npm audit-npm

CARGO := cargo
PROFIL ?= debug

# nextest applies the profile of .config/nextest.toml: serialized execution of
# the tests that share a server, and termination of a hanging test. It is not
# mandatory — a freshly cloned repository must be able to pass the gate without
# installing anything — but its absence changes what runs, and says so.
NEXTEST := $(shell command -v cargo-nextest 2>/dev/null)

# The front end of the Tauri application (ADR-0029). pnpm is not optional like
# nextest: without it, the front end would escape the gate, and `oxyn-desktop`
# would not compile — `tauri::generate_context!` reads `apps/desktop/dist/client`
# at compile time.
FRONT := apps/desktop
PNPM := $(shell command -v pnpm 2>/dev/null)
TAURI := $(FRONT)/node_modules/.bin/tauri

# The licenses of the shipped dependencies, which the application displays in
# Settings > About (ADR-0044). Outside git: it is regenerated when the Rust or
# npm graph changes, or when the list of accepted licenses changes.
MENTIONS := $(FRONT)/src/generated/third-party-licenses.json

aide:
	@echo "make qualite   the full quality gate"
	@echo "make verif-rapide  only checks what changed since origin/main (not conclusive)"
	@echo "make socle     checks the Claude and Codex foundation (usable without Rust code)"
	@echo "make hooks     replays the hook tests"
	@echo "make todo      refuses a remaining-work marker without a deadline"
	@echo "make versions  refuses a pinned version missing from docs/RESEARCH-NOTES.md"
	@echo "make front     checks the front end: npm licenses, format, lint, types, tests, stories, build"
	@echo "make rust      checks the Rust: format, clippy, tests, doc, dependencies, package contracts"
	@echo "make test-serveurs  runs the ignored database-server suites (requires their test URLs)"
	@echo "make desktop-dev  runs the Tauri application with hot reload"
	@echo "make desktop   builds the Tauri application (PROFIL=release to publish)"
	@echo ""
	@echo "script/nouvelle-crate <name> <description>   creates a compliant crate"

# --- The application ----------------------------------------------------------
# The development CSP is relaxed by tauri.dev.json5, and by it alone: a build
# never uses that file.
desktop-dev: $(TAURI)
	cd crates/oxyn-desktop && ../../$(TAURI) dev -c tauri.dev.json5 -- -- --temporary-workspace

# A published binary always ships its third-party notices, regenerated: without
# them, it would violate the licenses of what it redistributes. `--exiger` makes
# publishing fail if `cargo-about` is missing.
#
# No build signs the updater artifacts (ADR-0051): `tauri build` runs every
# build.rs and Vite plugin, which must never see the updater private key. The
# release workflow signs them afterwards, in a step of its own
# (`script/livraison signer`).
desktop: $(TAURI)
ifeq ($(PROFIL),release)
	@python3 script/licences-tierces generer --exiger $(MENTIONS)
	cd crates/oxyn-desktop && ../../$(TAURI) build
else
	cd crates/oxyn-desktop && ../../$(TAURI) build --debug --no-bundle
endif

$(TAURI):
	@test -n "$(PNPM)" || { echo "pnpm is required (apps/desktop/package.json, packageManager field)."; exit 1; }
	cd $(FRONT) && pnpm install --frozen-lockfile

# --- The gate -----------------------------------------------------------------
# As long as no Cargo.toml exists, the Rust targets are skipped and only the
# foundation is checked. Silence is not success: the message says so.
qualite: socle todo versions test-serveurs
ifeq ($(wildcard Cargo.toml),)
	@echo ""
	@echo "No Cargo.toml: the Rust checks did NOT run."
	@echo "This is not a full success — see docs/IMPLEMENTATION-PLAN.md, phase 0."
else
	@$(MAKE) --no-print-directory front rust
	@echo ""
	@echo "Quality gate passed."
endif

rust: format lint test doc deny package-contracts

# Outside the gate, deliberately: it sees neither the dependent crates nor what
# CI checks alone. The details are in the script.
verif-rapide:
	@python3 script/verif-rapide

# Licenses and published security advisories — the only check of the gate that
# looks at dependencies rather than code. `docs/SECURITY.md` § Dependencies
# makes it a promise; it is kept here, and its configuration lives in
# `deny.toml`.
#
# A local clone without the optional tool gets an explicit warning. CI has no
# such escape hatch: the workflow installs the pinned binary, and a missing
# installation must fail rather than turn the dependency check into a no-op.
deny:
ifeq ($(shell command -v cargo-deny 2>/dev/null),)
ifneq ($(origin CI),undefined)
	@echo "cargo-deny is required in CI; dependency checks did NOT run."
	@exit 1
endif
	@echo "cargo-deny missing: licenses and RUSTSEC advisories were NOT checked."
	@echo "  To install it: cargo install --locked cargo-deny"
else
	$(CARGO) deny --all-features check
endif

format:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

# nextest does not run documentation tests: `cargo test --doc` is therefore run
# in addition, otherwise the `//!` examples would stop being compiled without
# anything saying so.
test:
ifeq ($(NEXTEST),)
	@echo "cargo-nextest missing: the profile of .config/nextest.toml does not apply."
	$(CARGO) test --workspace --all-features
else
	$(CARGO) nextest run --workspace --all-features
	$(CARGO) test --doc --workspace --all-features
endif

# The ignored suites exercise actual server behavior that unit tests cannot
# emulate. They need disposable servers and URLs; their dedicated CI job
# provides both and rejects an unset URL rather than silently skipping a suite.
test-serveurs: front-build
ifeq ($(NEXTEST),)
	@echo "cargo-nextest is required for server tests; the shared-server profile must apply."
	@exit 1
else
	@output="$$( $(CARGO) nextest run --run-ignored only --nocapture \
		-E 'package(oxyn-driver-postgres) | package(oxyn-driver-mysql) | (package(oxyn-exec) & test(/production_read_tests::server::/)) | (package(oxyn-desktop) & test(/server_cancel::/))' 2>&1 )"; status=$$?; \
	printf '%s\n' "$$output"; \
	test $$status -eq 0; \
	if printf '%s\n' "$$output" | grep -Fq 'not set: test skipped'; then \
		echo "A server test URL is missing; refusing a silently skipped suite."; \
		exit 1; \
	fi
endif

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps --all-features

# The Apache driver contract is consumed outside this workspace. Package each
# crate so a missing dependency version fails before a release reaches crates.io.
package-contracts:
	$(CARGO) package -p oxyn-core -p oxyn-catalog -p oxyn-data -p oxyn-driver --no-verify

# Before the Rust targets, not after: the front-end build produces the
# directory `oxyn-desktop` embeds at compile time. A CI job that runs `rust`
# therefore runs `front-build` before it.
front: front-controles front-tests front-build

front-controles: $(TAURI) licences-npm audit-npm
	@python3 script/verifier-stories
	cd $(FRONT) && pnpm exec prettier --check .
	cd $(FRONT) && pnpm exec eslint .
	cd $(FRONT) && pnpm exec tsc --noEmit

# `SHARD=1/2` only runs the first half of the test files: CI spreads the stories
# over several runners that way. Without SHARD, everything runs.
front-tests: $(TAURI)
	cd $(FRONT) && pnpm exec playwright install chromium
	cd $(FRONT) && pnpm exec vitest run $(if $(SHARD),--shard=$(SHARD))

front-build: $(TAURI) $(MENTIONS)
	cd $(FRONT) && pnpm build

# Without `cargo-about`, warns without writing anything: the
# build goes through, and the About section says the notices are missing from
# this build.
$(MENTIONS): Cargo.lock $(FRONT)/pnpm-lock.yaml deny.toml $(FRONT)/licences-npm.toml script/licences-tierces
	@python3 script/licences-tierces generer $@

# The front-end counterpart of `make deny`: a production npm dependency whose
# license is not accepted by deny.toml (or, for npm-specific cases, by
# apps/desktop/licences-npm.toml) fails the gate.
licences-npm: $(TAURI)
	@python3 script/licences-tierces verifier-npm

# Published advisories for the npm graph shipped to users. Development-only
# tooling is excluded because it is not part of the desktop application.
audit-npm: $(TAURI)
	cd $(FRONT) && pnpm audit --prod

todo:
	@python3 script/verifier-todo

# Every version pinned in the root Cargo.toml and in apps/desktop/package.json
# must be recorded, dated, in docs/RESEARCH-NOTES.md (I-12). Offline on
# purpose: the gate cannot depend on the network. The comparison with the
# registries stays manual, through `/versions`.
versions:
	@python3 .claude/hooks/verifier_versions.py --manifests

# --- The foundation -----------------------------------------------------------
socle: hooks
	@python3 .claude/test_verifier_socle.py
	@python3 script/test_livraison.py
	@python3 script/test_apple_release.py
	@python3 script/test_verifier_todo.py
	@python3 .claude/verifier_socle.py

hooks:
	@python3 .claude/hooks/test_hooks.py
	@python3 .claude/hooks/test_verifier_versions.py
