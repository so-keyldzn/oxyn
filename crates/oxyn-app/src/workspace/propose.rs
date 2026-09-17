//! `Propose change…` : compose du SQL à relire, et n'exécute rien.
//!
//! La forme est celle de `related_row_query` dans [`super::metadata`], et pour
//! la même raison : un **modèle à compléter**, jamais une instruction à lancer.
//! Oxyn fournit ce qu'il connaît — la relation, la colonne, leur citation
//! correcte — et laisse à l'utilisateur ce qu'il est seul à savoir : le nouveau
//! nom, la nouvelle expression par défaut.
//!
//! La décision et ses raisons vivent dans
//! [ADR-0025](../../../../docs/adr/0025-proposition-de-changement-de-schema.md).
//! Ce qu'il faut en retenir ici : le texte part par
//! `open_library_query`, comme `Open DDL in console`, donc **aucun second chemin
//! d'exécution** ([I-01](../../../CLAUDE.md#i-01)).

use super::*;
use oxyn_catalog::CatalogCache;
use oxyn_catalog::path::{QuoteStyle, quote_identifier};
use oxyn_core::SqlDialect;

#[cfg(test)]
mod tests;

/// En-tête du modèle : ce que la maquette `229:7749` promet, dans le texte.
///
/// Rendue **sans** préfixe de commentaire : [`en_commentaire`] s'en charge pour
/// tout le modèle d'un seul geste. Écrite dans le texte plutôt qu'à l'écran
/// parce qu'elle doit survivre au copier-coller vers un ticket ou un message —
/// c'est là que la proposition sera relue, souvent par quelqu'un d'autre.
fn entete(relation: &str, connection: &str) -> String {
    format!(
        "Proposed change · {relation} · {connection}\n\
         Nothing has been executed. Uncomment one statement, complete it, and review it\n\
         before running: this connection is named above for that reason.\n"
    )
}

/// Commente **chaque ligne physique** du modèle.
///
/// # Pourquoi ce passage existe
///
/// `--` ne commente que jusqu'au prochain saut de ligne. Écrire les `--` à la
/// main dans les `format!` supposait qu'aucun fragment interpolé n'en contienne,
/// et cette supposition était fausse : `CatalogPath` rejette les caractères de
/// contrôle, mais `Field::new` et `Constraint::new` ne valident rien, et une
/// expression de défaut multi-ligne (`DEFAULT 'a` + saut + `b'::text`) est
/// banale. Une seule ligne échappée transformait un modèle annoncé comme
/// « Nothing has been executed » en instruction que `Run` exécute
/// ([I-10](../../../CLAUDE.md#i-10)).
///
/// Passer ici en dernier rend la garantie **structurelle** : elle ne dépend plus
/// de ce que contiennent les identifiants. Tenu par
/// `un_saut_de_ligne_dans_un_nom_ne_sort_pas_du_commentaire`.
///
/// # Pourquoi pas `oxyn_ai::untrusted::sanitize_inline`
///
/// Cette fonction existe et traite le même mode de panne — son `///` décrit
/// exactement ce cas. Elle **replie** le texte sur une seule ligne en
/// remplaçant les sauts par des espaces, ce qui est le bon parti là où elle
/// sert : un commentaire de colonne dans une invite IA, où la lisibilité prime.
///
/// Ici, non. Replier un saut de ligne contenu dans un **nom de colonne** en
/// espace produirait un modèle qui, une fois décommenté, désigne une colonne
/// **différente** — du SQL silencieusement faux, ce qui est pire que du SQL
/// illisible. Préfixer chaque ligne garde l'identifiant exact *et* commenté.
/// Les deux fonctions répondent donc à la même menace par deux partis opposés,
/// et chacun est juste à sa place.
fn en_commentaire(texte: &str) -> String {
    let mut sortie = texte
        .lines()
        .map(|ligne| {
            if ligne.is_empty() {
                "--".to_owned()
            } else {
                format!("-- {ligne}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    sortie.push('\n');
    sortie
}

/// Le nom qualifié de la relation, cité pour le dialecte.
fn relation_citee(path: &CatalogPath, style: QuoteStyle) -> Option<String> {
    let mut morceaux = Vec::new();
    if let Some(namespace) = path.namespace() {
        morceaux.push(quote_identifier(namespace, style));
    }
    morceaux.push(quote_identifier(path.relation()?, style));
    Some(morceaux.join("."))
}

/// Ce que le dialecte accepte de modifier sur une table existante.
///
/// **Ce n'est pas une prudence, c'est une contrainte du moteur.** SQLite ne
/// connaît d'`ALTER TABLE` que `RENAME TO`, `RENAME COLUMN`, `ADD COLUMN` et
/// `DROP COLUMN` : il n'a ni `ALTER COLUMN`, ni `DROP CONSTRAINT`. Proposer une
/// nullabilité ou un défaut y produirait un modèle qui échoue à l'exécution,
/// c'est-à-dire exactement la promesse creuse qu'[ADR-0003](../../../../docs/adr/0003-driver-capabilities.md)
/// interdit de faire.
const fn accepte_alter_column(dialect: SqlDialect) -> bool {
    matches!(dialect, SqlDialect::Postgres | SqlDialect::Redshift)
}

/// Y a-t-il un modèle à proposer, sans le composer ?
///
/// Doit répondre **exactement** comme `proposed_change(...).is_some()` : un
/// bouton visible dont l'activation ne produit rien serait pire qu'aucun bouton.
/// Tenu par `le_bouton_existe_exactement_quand_un_modele_existe`.
#[must_use]
pub(super) fn peut_proposer(
    cache: &CatalogCache,
    path: &CatalogPath,
    tab: ObjectTab,
    index: usize,
    dialect: SqlDialect,
) -> bool {
    if path.relation().is_none() {
        return false;
    }
    match tab {
        ObjectTab::Structure => cache
            .relation(path)
            .is_some_and(|relation| relation.fields.get(index).is_some()),
        ObjectTab::Constraints => {
            accepte_alter_column(dialect)
                && cache
                    .constraints(path)
                    .is_some_and(|contraintes| contraintes.get(index).is_some())
        }
        _ => false,
    }
}

/// Le modèle de changement pour la ligne sélectionnée de l'onglet courant.
///
/// Rend `None` quand il n'y a rien à proposer : onglet sans objet modifiable,
/// sélection hors bornes, ou dialecte qui n'accepte pas l'opération. `None`
/// éteint le contrôle — mieux vaut pas de bouton qu'un bouton qui produit un
/// texte inexécutable.
///
/// Les identifiants viennent du catalogue, donc du serveur, et sont **cités**
/// ([I-10](../../../CLAUDE.md#i-10)). Une colonne nommée
/// `"x"; DROP TABLE audit; --` est légale dans PostgreSQL ; concaténée ici, elle
/// produirait un modèle qui supprime une table au premier clic sur `Run`.
pub(super) fn proposed_change(
    cache: &CatalogCache,
    path: &CatalogPath,
    tab: ObjectTab,
    index: usize,
    dialect: SqlDialect,
    connection: &str,
) -> Option<String> {
    let style = QuoteStyle::for_dialect(dialect);
    let table = relation_citee(path, style)?;
    let entete = entete(&table, connection);

    match tab {
        ObjectTab::Structure => {
            let champ = cache.relation(path)?.fields.get(index)?.clone();
            let colonne = quote_identifier(&champ.name, style);
            // Le renommage est la seule opération que les deux dialectes
            // partagent ; elle vient donc en premier et existe toujours.
            let mut lignes = format!(
                "{entete}\n\
                 Rename the column. Replace new_name before running.\n\
                 ALTER TABLE {table} RENAME COLUMN {colonne} TO new_name;\n"
            );
            if accepte_alter_column(dialect) {
                // On ne propose que le sens qui **change** l'état actuel :
                // offrir les deux laisserait choisir celui qui ne fait rien, et
                // un modèle qui ne fait rien se lit comme un modèle qui a échoué.
                let nullabilite = if champ.nullable {
                    format!("ALTER TABLE {table} ALTER COLUMN {colonne} SET NOT NULL;")
                } else {
                    format!("ALTER TABLE {table} ALTER COLUMN {colonne} DROP NOT NULL;")
                };
                lignes.push_str(&format!(
                    "\nChange nullability. The column is currently {}.\n{nullabilite}\n",
                    if champ.nullable {
                        "nullable"
                    } else {
                        "NOT NULL"
                    }
                ));
                // Le défaut existant est repris **verbatim** du catalogue, en
                // commentaire : le reformater changerait son sens sans le dire.
                match champ.default.as_deref() {
                    Some(actuel) => lignes.push_str(&format!(
                        "\nChange or remove the default. It is currently: {actuel}\n\
                         ALTER TABLE {table} ALTER COLUMN {colonne} SET DEFAULT new_expression;\n\
                         ALTER TABLE {table} ALTER COLUMN {colonne} DROP DEFAULT;\n"
                    )),
                    None => lignes.push_str(&format!(
                        "\nAdd a default. The column has none.\n\
                         ALTER TABLE {table} ALTER COLUMN {colonne} SET DEFAULT new_expression;\n"
                    )),
                }
            }
            Some(en_commentaire(&lignes))
        }
        ObjectTab::Constraints => {
            if !accepte_alter_column(dialect) {
                return None;
            }
            let contrainte = cache.constraints(path)?.get(index)?.clone();
            let nom = quote_identifier(&contrainte.name, style);
            Some(en_commentaire(&format!(
                "{entete}\n\
                 Drop the constraint. This does not remove the data it protected.\n\
                 ALTER TABLE {table} DROP CONSTRAINT {nom};\n"
            )))
        }
        // Les autres onglets ne portent pas d'objet que cette portée sait
        // modifier — voir ADR-0025 : ni DROP TABLE, ni changement de type, ni
        // migration de données.
        _ => None,
    }
}

impl Workspace {
    /// Y a-t-il quelque chose à proposer sur la sélection courante ?
    ///
    /// Séparée de la composition parce qu'elle est appelée **au rendu**, une fois
    /// par trame de l'onglet `Structure`. Composer le modèle entier pour en jeter
    /// le résultat coûterait un clone de champ et plusieurs `format!` par trame ;
    /// ce n'est ni une I/O ni un blocage, donc pas une violation d'I-05, mais
    /// c'est du travail par trame proportionnel au modèle.
    ///
    /// Les deux fonctions doivent répondre sur les **mêmes** conditions : un
    /// bouton visible dont l'activation ne produit rien serait pire qu'aucun
    /// bouton. C'est ce que tient
    /// `le_bouton_existe_exactement_quand_un_modele_existe`.
    pub(super) fn can_propose_change(&self) -> bool {
        let Some(path) = self.selected_path.as_ref() else {
            return false;
        };
        let Some(cache) = self.catalog_cache.try_read() else {
            return false;
        };
        peut_proposer(
            &cache,
            path,
            self.object_tab,
            self.metadata_selected,
            self.dialect,
        )
    }

    /// Le modèle courant, s'il y en a un à proposer.
    ///
    /// Appelée au rendu pour décider si le contrôle existe, et à l'activation
    /// pour le composer : passer par le même chemin est ce qui garantit que le
    /// bouton visible et le texte produit parlent du même objet.
    pub(super) fn proposed_change_text(&self) -> Option<String> {
        let path = self.selected_path.as_ref()?;
        let cache = self.catalog_cache.try_read()?;
        proposed_change(
            &cache,
            path,
            self.object_tab,
            self.metadata_selected,
            self.dialect,
            &self.display.name,
        )
    }

    /// `Propose change…` de `229:7663` : ouvre le modèle dans une console.
    ///
    /// Rien ne s'exécute. Le texte part par `open_library_query`, le chemin que
    /// `Open DDL in console` emprunte déjà, et la relecture qui suit est celle
    /// que la maquette promet — « a SQL review naming commerce-prod before
    /// execution » ([ADR-0025](../../../../docs/adr/0025-proposition-de-changement-de-schema.md)).
    pub(super) fn open_proposed_change(&mut self, cx: &mut Context<'_, Self>) {
        let Some(texte) = self.proposed_change_text() else {
            return;
        };
        self.open_library_query(
            library::OpenQuery::Copy {
                text: texte,
                title: "Proposed change.sql".into(),
                origin: "a schema-change template · uncomment and complete one statement before \
                         running"
                    .into(),
                // Même raison que le modèle de requête liée : un squelette
                // composé par Oxyn n'est écrit ni par l'utilisateur ni par un
                // agent, et la provenance marque qui a écrit.
                provenance: None,
            },
            cx,
        );
    }
}
