//! Les paramètres liés : `ScalarValue` → classe de stockage SQLite.
//!
//! **Les valeurs se lient, elles ne se concatènent pas**
//! ([`DRIVER-CONTRACT` §6](../../../docs/DRIVER-CONTRACT.md)). Rien dans ce
//! module ne produit du texte de requête : chaque valeur passe par
//! `sqlite3_bind_*`.
//!
//! # La table de correspondance, et ce qu'elle perd
//!
//! SQLite n'a que cinq classes de stockage — NULL, INTEGER, REAL, TEXT, BLOB.
//! Tout le reste est une **convention d'encodage**, et une convention se
//! documente :
//!
//! | `ScalarValue` | Classe SQLite | Ce qui se perd |
//! |---|---|---|
//! | `Null` | NULL | rien |
//! | `Bool` | INTEGER `0`/`1` | le type : `1` et `true` sont indiscernables à la relecture |
//! | `Int64` | INTEGER | rien |
//! | `Float64` | REAL | rien |
//! | `Decimal` | TEXT | l'ordre : la comparaison devient lexicographique. La **valeur** est exacte, ce qui est le point : aucun flottant ne représente `0.10` |
//! | `Text` | TEXT | rien |
//! | `Bytes` | BLOB | rien |
//! | `Uuid` | TEXT canonique à tirets | le type ; c'est la convention SQLite usuelle |
//! | `Date` | TEXT `AAAA-MM-JJ` | le type ; format des fonctions `date()` de SQLite |
//! | `Time` | TEXT `HH:MM:SS[.fff]` | le type |
//! | `Timestamp` | TEXT RFC 3339 **en UTC** | le type ; le fuseau est conservé, jamais converti au fuseau du poste |
//! | `TimestampNaive` | TEXT `AAAA-MM-JJ HH:MM:SS` | le type ; **aucun fuseau n'est inventé** |
//! | `Json` | TEXT | le type ; c'est ce qu'attend l'extension `json1` |
//! | `Interval` | **refusé** | — |
//! | `Array` | **refusé** | — |
//!
//! Les deux refus ne sont pas des trous à combler : SQLite n'a ni intervalle ni
//! tableau, et les encoder en texte produirait une valeur qu'aucune requête ne
//! saurait relire. Ne pas savoir faire est une réponse acceptable ; laisser
//! croire ne l'est pas.

use oxyn_core::ScalarValue;
use rusqlite::Statement;
use rusqlite::types::Value;

use crate::error::SqliteError;

/// Lie les paramètres positionnels d'une instruction préparée.
///
/// Les emplacements SQLite sont numérotés **à partir de 1** ; `params[0]`
/// alimente donc `?1`.
///
/// # Erreurs
/// [`SqliteError::ParameterCount`] si le compte ne correspond pas à ce
/// qu'attend l'instruction, [`SqliteError::Parameter`] pour une valeur que
/// SQLite ne sait pas ranger, ou l'erreur du moteur si la liaison échoue.
pub(crate) fn bind(
    statement: &mut Statement<'_>,
    params: &[ScalarValue],
) -> Result<(), SqliteError> {
    let expected = statement.parameter_count();
    if expected != params.len() {
        return Err(SqliteError::ParameterCount {
            expected,
            given: params.len(),
        });
    }
    for (position, value) in params.iter().enumerate() {
        // Le compte est vérifié ci-dessus, et `enumerate` ne déborde pas :
        // `position + 1` tient dans un `usize` puisque `position < len`.
        let index = position.saturating_add(1);
        statement.raw_bind_parameter(index, to_storage(value, index)?)?;
    }
    Ok(())
}

/// Convertit une valeur du domaine en classe de stockage SQLite.
///
/// Le `match` est **exhaustif à dessein** : [`ScalarValue`] est une énumération
/// fermée précisément pour qu'ajouter un type scalaire fasse échouer la
/// compilation de chaque driver, et pose la question « et celui-là, je le rends
/// comment ? » plutôt qu'un `_ =>` qui déciderait en silence.
fn to_storage(value: &ScalarValue, index: usize) -> Result<Value, SqliteError> {
    let refuse = || SqliteError::Parameter {
        index,
        type_name: value.type_name(),
    };
    Ok(match value {
        ScalarValue::Null => Value::Null,
        ScalarValue::Bool(b) => Value::Integer(i64::from(*b)),
        ScalarValue::Int64(i) => Value::Integer(*i),
        ScalarValue::Float64(x) => Value::Real(*x),
        // Le texte, et non un flottant : aucun `f64` ne représente `0.10`, et une
        // valeur monétaire arrondie au passage est une corruption silencieuse.
        ScalarValue::Decimal(d) => Value::Text(d.clone()),
        ScalarValue::Text(t) => Value::Text(t.clone()),
        ScalarValue::Bytes(b) => Value::Blob(b.clone()),
        ScalarValue::Uuid(u) => Value::Text(u.to_string()),
        ScalarValue::Date(d) => Value::Text(d.to_string()),
        ScalarValue::Time(t) => Value::Text(t.to_string()),
        // RFC 3339, en UTC. Convertir vers le fuseau du poste décalerait la
        // donnée de façon invisible et permanente (DRIVER-CONTRACT §7).
        ScalarValue::Timestamp(ts) => Value::Text(ts.to_rfc3339()),
        // `AAAA-MM-JJ HH:MM:SS`, la forme que rendent les fonctions `datetime()`
        // de SQLite. Aucun fuseau n'y est ajouté.
        ScalarValue::TimestampNaive(ts) => Value::Text(ts.to_string()),
        ScalarValue::Json(v) => Value::Text(v.to_string()),
        // SQLite n'a ni intervalle ni tableau. Les encoder en texte produirait
        // une valeur qu'aucune requête SQLite ne saurait relire.
        ScalarValue::Interval { .. } | ScalarValue::Array(_) => return Err(refuse()),
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn stocke(value: &ScalarValue) -> Result<Value, SqliteError> {
        to_storage(value, 1)
    }

    #[test]
    fn un_booleen_devient_un_entier() {
        // SQLite n'a pas de type booléen : `0` et `1` sont la convention.
        assert_eq!(
            stocke(&ScalarValue::Bool(true)).expect("lié"),
            Value::Integer(1)
        );
        assert_eq!(
            stocke(&ScalarValue::Bool(false)).expect("lié"),
            Value::Integer(0)
        );
    }

    #[test]
    fn une_decimale_ne_passe_pas_par_un_flottant() {
        let value = stocke(&ScalarValue::Decimal("0.10".to_owned())).expect("lié");
        assert_eq!(
            value,
            Value::Text("0.10".to_owned()),
            "les zéros de queue sont signifiants, et aucun f64 ne tient 0.10"
        );
    }

    #[test]
    fn les_octets_restent_des_octets() {
        // Un BLOB ne se rend pas en texte « au mieux » : il reste opaque.
        let value = stocke(&ScalarValue::Bytes(vec![0x00, 0xff, 0x80])).expect("lié");
        assert_eq!(value, Value::Blob(vec![0x00, 0xff, 0x80]));
    }

    #[test]
    fn un_intervalle_et_un_tableau_sont_refuses_pas_encodes() {
        // SQLite n'a ni l'un ni l'autre ; les encoder produirait une valeur
        // qu'aucune requête ne saurait relire.
        for valeur in [
            ScalarValue::Interval {
                months: 1,
                days: 0,
                nanos: 0,
            },
            ScalarValue::Array(vec![ScalarValue::Int64(1)]),
        ] {
            let err = stocke(&valeur).expect_err("refus attendu");
            assert!(
                matches!(err, SqliteError::Parameter { .. }),
                "{err:?} pour {}",
                valeur.type_name()
            );
        }
    }

    #[test]
    fn un_compte_de_parametres_faux_est_refuse_avant_toute_execution() {
        let conn = Connection::open_in_memory().expect("base en mémoire");
        let mut stmt = conn.prepare("SELECT ?1, ?2").expect("préparation");

        let err = bind(&mut stmt, &[ScalarValue::Int64(1)]).expect_err("refus attendu");
        let SqliteError::ParameterCount { expected, given } = err else {
            panic!("mauvaise variante : {err:?}");
        };
        assert_eq!((expected, given), (2, 1));
    }

    #[test]
    fn une_valeur_liee_ne_traverse_jamais_le_texte_de_la_requete() {
        // Le test qui compte : une valeur hostile liée reste une valeur.
        let conn = Connection::open_in_memory().expect("base en mémoire");
        conn.execute_batch("CREATE TABLE audit(note TEXT); CREATE TABLE t(v TEXT);")
            .expect("schéma");

        let mut stmt = conn
            .prepare("INSERT INTO t(v) VALUES (?1)")
            .expect("préparation");
        bind(
            &mut stmt,
            &[ScalarValue::Text("'); DROP TABLE audit; --".to_owned())],
        )
        .expect("liaison");
        stmt.raw_execute().expect("insertion");

        let reste: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'audit'",
                [],
                |row| row.get(0),
            )
            .expect("compte");
        assert_eq!(reste, 1, "la table d'audit a été supprimée");
    }

    // Les variantes temporelles, `Uuid` et `Json` de `ScalarValue` ne sont pas
    // couvertes ici : les construire demanderait `chrono`, `uuid` et
    // `serde_json`, qui ne sont pas au contrat de dépendances de cette crate.
    // Leur rendu est décrit dans la table du module et n'utilise que le
    // `Display` de chaque type — donc rien qui puisse diverger en silence.
}
