# La porte de qualité du dépôt.
#
# `make qualite` est le SEUL point d'entrée : la CI l'appelle, le hook Stop le
# rappelle, la définition de « terminé » s'y adosse. Un contrôle ajouté ailleurs
# est un contrôle optionnel.
#
# Voir .claude/rules/manifestes.md et .claude/checklists/fin-de-tache.md.

.PHONY: qualite format lint test doc hooks socle aide

CARGO := cargo

aide:
	@echo "make qualite   la porte de qualité complète"
	@echo "make socle     vérifie le socle .claude/ (utilisable sans code Rust)"
	@echo "make hooks     rejoue les tests des hooks"

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
