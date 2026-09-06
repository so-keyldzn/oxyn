# La porte de qualité du dépôt.
#
# `make qualite` est le SEUL point d'entrée : la CI l'appelle, le hook Stop le
# rappelle, la définition de « terminé » s'y adosse. Un contrôle ajouté ailleurs
# est un contrôle optionnel.
#
# Voir .claude/rules/manifestes.md et .claude/checklists/fin-de-tache.md.

.PHONY: qualite format lint test doc hooks socle aide app lancer

CARGO := cargo
PROFIL ?= debug
APP := target/$(PROFIL)/Oxyn.app

aide:
	@echo "make qualite   la porte de qualité complète"
	@echo "make socle     vérifie le socle .claude/ (utilisable sans code Rust)"
	@echo "make hooks     rejoue les tests des hooks"
	@echo "make app       assemble target/\$$PROFIL/Oxyn.app (PROFIL=release pour publier)"
	@echo "make lancer    assemble puis ouvre l'application"

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
qualite: socle
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

test:
	$(CARGO) test --workspace --all-features

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps --all-features

# --- Le socle -----------------------------------------------------------------
socle: hooks
	@python3 .claude/verifier_socle.py

hooks:
	@python3 .claude/hooks/test_hooks.py
