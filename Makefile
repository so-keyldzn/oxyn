# La porte de qualité du dépôt.
#
# `make qualite` est le SEUL point d'entrée : le hook Stop le rappelle, la
# définition de « terminé » s'y adosse. Un contrôle ajouté ailleurs est un
# contrôle optionnel.
#
# La CI (.github/workflows/qualite.yml) en appelle les morceaux dans des jobs
# parallèles — `front-controles`, `front-tests`, `front-build`, `rust` — pour ne
# pas faire attendre le Rust derrière les stories. Elle n'ajoute rien : `make
# socle` refuse un workflow qui oublierait une cible atteinte par `qualite`.
#
# Voir .claude/rules/manifestes.md et .claude/checklists/fin-de-tache.md.

.PHONY: qualite format lint test doc deny todo hooks socle aide front front-controles front-tests front-build rust desktop desktop-dev licences-npm

CARGO := cargo
PROFIL ?= debug

# nextest applique le profil de .config/nextest.toml : exécution sérialisée des
# tests qui partagent un serveur, et arrêt d'un test qui pend. Il n'est pas
# obligatoire — un dépôt fraîchement cloné doit pouvoir passer la porte sans
# rien installer — mais son absence change ce qui tourne, et se dit.
NEXTEST := $(shell command -v cargo-nextest 2>/dev/null)

# Le front de l'application Tauri (ADR-0029). pnpm n'est pas optionnel comme
# nextest : sans lui, le front échapperait à la porte, et `oxyn-desktop` ne
# compilerait pas — `tauri::generate_context!` lit `apps/desktop/dist/client`
# à la compilation.
FRONT := apps/desktop
PNPM := $(shell command -v pnpm 2>/dev/null)
TAURI := $(FRONT)/node_modules/.bin/tauri

# Les licences des dépendances livrées, que l'application affiche dans
# Settings > About (ADR-0044). Hors de git : il se régénère quand le graphe
# Rust ou npm change, ou quand la liste des licences acceptées change.
MENTIONS := $(FRONT)/src/generated/third-party-licenses.json

aide:
	@echo "make qualite   la porte de qualité complète"
	@echo "make socle     vérifie le socle Claude et Codex (utilisable sans code Rust)"
	@echo "make hooks     rejoue les tests des hooks"
	@echo "make todo      refuse une marque de travail restant sans échéance"
	@echo "make front     contrôle le front : licences npm, format, lint, types, tests, stories, build"
	@echo "make rust      contrôle le Rust : format, clippy, tests, doc, dépendances"
	@echo "make desktop-dev  lance l'application Tauri avec rechargement à chaud"
	@echo "make desktop   construit l'application Tauri (PROFIL=release pour publier)"
	@echo ""
	@echo "script/nouvelle-crate <nom> <description>   crée une crate conforme"

# --- L'application ------------------------------------------------------------
# La CSP de développement est relâchée par tauri.dev.json5, et par lui seul :
# un build n'utilise jamais ce fichier.
desktop-dev: $(TAURI)
	cd crates/oxyn-desktop && ../../$(TAURI) dev -c tauri.dev.json5 -- -- --temporary-workspace

# Un binaire publié embarque toujours ses mentions tierces, régénérées : sans
# elles, il violerait les licences de ce qu'il redistribue. `--exiger` fait
# échouer la publication si `cargo-about` manque.
desktop: $(TAURI)
ifeq ($(PROFIL),release)
	@python3 script/licences-tierces generer --exiger $(MENTIONS)
	cd crates/oxyn-desktop && ../../$(TAURI) build
else
	cd crates/oxyn-desktop && ../../$(TAURI) build --debug --no-bundle
endif

$(TAURI):
	@test -n "$(PNPM)" || { echo "pnpm est requis (apps/desktop/package.json, champ packageManager)."; exit 1; }
	cd $(FRONT) && pnpm install --frozen-lockfile

# --- La porte -----------------------------------------------------------------
# Tant qu'aucun Cargo.toml n'existe, les cibles Rust sont ignorées et seul le
# socle est vérifié. Le silence n'est pas un succès : le message le dit.
qualite: socle todo
ifeq ($(wildcard Cargo.toml),)
	@echo ""
	@echo "Aucun Cargo.toml : les contrôles Rust n'ont PAS tourné."
	@echo "Ce n'est pas un succès complet — voir docs/IMPLEMENTATION-PLAN.md, phase 0."
else
	@$(MAKE) --no-print-directory front rust
	@echo ""
	@echo "Porte de qualité franchie."
endif

rust: format lint test doc deny

# Licences et avis de sécurité publiés — le seul contrôle de la porte qui
# regarde les dépendances plutôt que le code. `docs/SECURITY.md` § Dépendances en
# fait une promesse ; elle est tenue ici, et sa configuration vit dans
# `deny.toml`.
#
# L'outil absent **avertit sans bloquer**, et c'est délibéré : une porte qui
# échoue faute d'un binaire optionnel finit par être contournée — et c'est alors
# tout le contrôle qui disparaît, pas seulement celui-ci. Le message dit ce qui
# n'a pas tourné, parce qu'un silence se lit comme un succès.
deny:
ifeq ($(shell command -v cargo-deny 2>/dev/null),)
	@echo "cargo-deny absent : les licences et les avis RUSTSEC n'ont PAS été vérifiés."
	@echo "  Pour l'installer : cargo install --locked cargo-deny"
else
	$(CARGO) deny --all-features check
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

# Avant les cibles Rust, et pas après : le build du front produit le dossier que
# `oxyn-desktop` embarque à la compilation. Un job de CI qui lance `rust` lance
# donc `front-build` avant lui.
front: front-controles front-tests front-build

front-controles: $(TAURI) licences-npm
	@python3 script/verifier-stories
	cd $(FRONT) && pnpm exec prettier --check .
	cd $(FRONT) && pnpm exec eslint .
	cd $(FRONT) && pnpm exec tsc --noEmit

# `SHARD=1/2` ne lance que la première moitié des fichiers de test : la CI
# répartit ainsi les stories sur plusieurs runners. Sans SHARD, tout tourne.
front-tests: $(TAURI)
	cd $(FRONT) && pnpm exec playwright install chromium
	cd $(FRONT) && pnpm exec vitest run $(if $(SHARD),--shard=$(SHARD))

front-build: $(TAURI) $(MENTIONS)
	cd $(FRONT) && pnpm build

# Sans `cargo-about`, avertit sans rien écrire, comme `make deny` : le build se
# fait, et la section About dit que les mentions manquent à ce build.
$(MENTIONS): Cargo.lock $(FRONT)/pnpm-lock.yaml deny.toml $(FRONT)/licences-npm.toml script/licences-tierces
	@python3 script/licences-tierces generer $@

# Le pendant de `make deny` pour le front : une dépendance npm de production
# dont la licence n'est pas acceptée par deny.toml (ou, cas propres à npm, par
# apps/desktop/licences-npm.toml) fait échouer la porte.
licences-npm: $(TAURI)
	@python3 script/licences-tierces verifier-npm

todo:
	@python3 script/verifier-todo

# --- Le socle -----------------------------------------------------------------
socle: hooks
	@python3 .claude/test_verifier_socle.py
	@python3 script/test_livraison.py
	@python3 .claude/verifier_socle.py

hooks:
	@python3 .claude/hooks/test_hooks.py
