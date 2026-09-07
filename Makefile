# La porte de qualité du dépôt.
#
# `make qualite` est le SEUL point d'entrée : la CI l'appelle
# (.github/workflows/qualite.yml), le hook Stop le rappelle, la définition de
# « terminé » s'y adosse. Un contrôle ajouté ailleurs est un contrôle optionnel.
#
# Voir .claude/rules/manifestes.md et .claude/checklists/fin-de-tache.md.

.PHONY: qualite format lint test doc todo hooks socle aide app lancer

CARGO := cargo
PROFIL ?= debug
APP := target/$(PROFIL)/Oxyn.app

# nextest applique le profil de .config/nextest.toml : exécution sérialisée des
# tests qui partagent un serveur, et arrêt d'un test qui pend. Il n'est pas
# obligatoire — un dépôt fraîchement cloné doit pouvoir passer la porte sans
# rien installer — mais son absence change ce qui tourne, et se dit.
NEXTEST := $(shell command -v cargo-nextest 2>/dev/null)

aide:
	@echo "make qualite   la porte de qualité complète"
	@echo "make socle     vérifie le socle Claude et Codex (utilisable sans code Rust)"
	@echo "make hooks     rejoue les tests des hooks"
	@echo "make todo      refuse une marque de travail restant sans échéance"
	@echo "make app       assemble target/\$$PROFIL/Oxyn.app (PROFIL=release pour publier)"
	@echo "make lancer    assemble puis ouvre l'application"
	@echo ""
	@echo "script/nouvelle-crate <nom> <description>   crée une crate conforme"

# --- L'application ------------------------------------------------------------
# Un binaire nu lancé depuis un terminal n'a pas d'identifiant de paquet : macOS
# le traite comme un accessoire, sans Dock ni activation propre, et aucun outil
# ne sait le désigner. Le paquet est la forme normale d'une application macOS,
# pas une étape de publication.
app:
ifeq ($(PROFIL),release)
	$(CARGO) build --release -p oxyn
else
	$(CARGO) build -p oxyn
endif
	@rm -rf "$(APP)"
	@mkdir -p "$(APP)/Contents/MacOS" "$(APP)/Contents/Resources"
	@cp crates/oxyn-app/Oxyn.app.plist "$(APP)/Contents/Info.plist"
	@cp "target/$(PROFIL)/oxyn" "$(APP)/Contents/MacOS/oxyn"
	@cp assets/brand/Oxyn.icns "$(APP)/Contents/Resources/Oxyn.icns"
	@printf 'APPL????' > "$(APP)/Contents/PkgInfo"
	@echo "$(APP)"

lancer: app
	@open "$(APP)"

# --- La porte -----------------------------------------------------------------
# Tant qu'aucun Cargo.toml n'existe, les cibles Rust sont ignorées et seul le
# socle est vérifié. Le silence n'est pas un succès : le message le dit.
qualite: socle todo
ifeq ($(wildcard Cargo.toml),)
	@echo ""
	@echo "Aucun Cargo.toml : les contrôles Rust n'ont PAS tourné."
	@echo "Ce n'est pas un succès complet — voir docs/IMPLEMENTATION-PLAN.md, phase 0."
else
	@$(MAKE) --no-print-directory format lint test doc
	@echo ""
	@echo "Porte de qualité franchie."
endif

format:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

# nextest n'exécute pas les tests de documentation : `cargo test --doc` est donc
# lancé en plus, sans quoi les exemples des `//!` cesseraient d'être compilés
# sans que rien ne le dise.
test:
ifeq ($(NEXTEST),)
	@echo "cargo-nextest absent : le profil de .config/nextest.toml ne s'applique pas."
	$(CARGO) test --workspace --all-features
else
	$(CARGO) nextest run --workspace --all-features
	$(CARGO) test --doc --workspace --all-features
endif

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps --all-features

todo:
	@python3 script/verifier-todo

# --- Le socle -----------------------------------------------------------------
socle: hooks
	@python3 .claude/verifier_socle.py

hooks:
	@python3 .claude/hooks/test_hooks.py
