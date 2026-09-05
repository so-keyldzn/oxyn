//! Le fichier de débordement d'un [`ResultBuffer`](crate::ResultBuffer).
//!
//! # Pourquoi un flux IPC par lot, et pas un fichier IPC
//!
//! Le format *fichier* d'Arrow IPC place son index de blocs dans un **pied de
//! page**, écrit par `FileWriter::finish()`. Tant que la requête n'est pas
//! terminée, ce pied n'existe pas : rien de ce qui a débordé ne serait
//! relisible — c'est-à-dire précisément pendant que l'utilisateur fait défiler.
//!
//! Chaque lot débordé est donc écrit comme un **flux IPC autonome** (message de
//! schéma, message de données, marqueur de fin) à un décalage relevé. Relire le
//! lot *i* est un `seek` puis une lecture de `len` octets, quel que soit l'état
//! d'avancement de la requête. Le surcoût est le message de schéma répété — de
//! l'ordre de la centaine d'octets face à un lot qui pèse des mégaoctets, faute
//! de quoi il n'aurait pas débordé.
//!
//! Le fichier est [`tempfile::NamedTempFile`] : il est supprimé à la destruction
//! du tampon, y compris en cas de panique. Un handle de lecture **indépendant**
//! est ouvert à la création pour que le défilement ne se sérialise pas derrière
//! l'écriture du lot suivant.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use arrow::datatypes::SchemaRef;
use arrow::ipc::reader::StreamReader;
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use parking_lot::Mutex;
use tempfile::{Builder, NamedTempFile};

use crate::error::{DataError, Result};

/// Où se trouve un lot débordé dans le fichier temporaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpillRef {
    /// Décalage du premier octet du flux IPC.
    offset: u64,
    /// Longueur du flux, marqueur de fin compris.
    len: u64,
    /// Lignes attendues à la relecture. Sert de contrôle de cohérence.
    rows: usize,
}

impl SpillRef {
    /// Octets occupés dans le fichier de débordement.
    pub(crate) const fn byte_len(self) -> u64 {
        self.len
    }
}

/// Côté écriture : un seul écrivain à la fois, position suivante mémorisée.
#[derive(Debug)]
struct WriteSide {
    file: NamedTempFile,
    next_offset: u64,
}

/// Le fichier de débordement, partagé par `Arc` entre l'écrivain (le puits) et
/// les lecteurs (la grille).
#[derive(Debug)]
pub(crate) struct SpillFile {
    write: Mutex<WriteSide>,
    /// Handle distinct du précédent : deux descripteurs sur le même fichier
    /// partagent le cache de pages, donc une écriture est visible d'une lecture
    /// sans `fsync` — et une lecture n'attend pas la fin d'une écriture.
    read: Mutex<File>,
}

impl SpillFile {
    /// Crée le fichier temporaire et son handle de lecture.
    ///
    /// Le préfixe rend le fichier identifiable dans un `lsof` : un fichier
    /// temporaire anonyme de plusieurs gigaoctets qu'on ne sait pas attribuer
    /// est un incident de support garanti.
    pub(crate) fn create() -> Result<Self> {
        let file = Builder::new()
            .prefix("oxyn-result-")
            .suffix(".arrows")
            .tempfile()
            .map_err(DataError::Spill)?;
        let read = file.reopen().map_err(DataError::Spill)?;
        Ok(Self {
            write: Mutex::new(WriteSide {
                file,
                next_offset: 0,
            }),
            read: Mutex::new(read),
        })
    }

    /// Écrit un lot à la suite et rend sa localisation.
    ///
    /// Bloque le temps de l'écriture. **À n'appeler ni depuis le thread
    /// d'interface, ni en tenant le verrou d'index du tampon**
    /// ([I-05](../../../CLAUDE.md#i-05)).
    pub(crate) fn append(&self, schema: &SchemaRef, batch: &RecordBatch) -> Result<SpillRef> {
        let mut side = self.write.lock();
        let offset = side.next_offset;
        let file = side.file.as_file_mut();
        file.seek(SeekFrom::Start(offset))
            .map_err(DataError::Spill)?;

        let mut writer = StreamWriter::try_new(&mut *file, schema.as_ref())?;
        writer.write(batch)?;
        writer.finish()?;
        drop(writer);

        let end = file.stream_position().map_err(DataError::Spill)?;
        side.next_offset = end;

        Ok(SpillRef {
            offset,
            len: end.saturating_sub(offset),
            rows: batch.num_rows(),
        })
    }

    /// Relit un lot débordé.
    ///
    /// Une lecture, jamais une réexécution de la requête
    /// ([PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-de-mémoire)).
    ///
    /// Alloue un tampon de la taille du lot : `memmap2` l'éviterait, mais son
    /// API est `unsafe` et le lint `unsafe_code = "deny"` du workspace
    /// l'interdit. Voir la note en tête de [`crate`].
    pub(crate) fn read(&self, reference: SpillRef) -> Result<RecordBatch> {
        let taille = usize::try_from(reference.len).map_err(|_| DataError::Spill(oversized()))?;
        let mut octets = vec![0_u8; taille];
        {
            let mut file = self.read.lock();
            file.seek(SeekFrom::Start(reference.offset))
                .map_err(DataError::Spill)?;
            file.read_exact(&mut octets).map_err(DataError::Spill)?;
        }

        let mut lecteur = StreamReader::try_new(octets.as_slice(), None)?;
        let lot = lecteur
            .next()
            .transpose()?
            .ok_or_else(|| DataError::Spill(truncated()))?;

        // Le fichier est le nôtre, mais un disque plein ou un système de
        // fichiers menteur produit un flux tronqué qui se décode quand même :
        // sans ce contrôle, la grille afficherait silencieusement moins de
        // lignes qu'annoncé par `locate`.
        if lot.num_rows() != reference.rows {
            return Err(DataError::Spill(inconsistent(
                reference.rows,
                lot.num_rows(),
            )));
        }
        Ok(lot)
    }
}

fn oversized() -> std::io::Error {
    std::io::Error::other("spilled batch is larger than this platform's address space")
}

fn truncated() -> std::io::Error {
    std::io::Error::other("spilled batch is missing from the temporary file")
}

fn inconsistent(attendu: usize, trouve: usize) -> std::io::Error {
    std::io::Error::other(format!(
        "spilled batch has {trouve} rows, expected {attendu}"
    ))
}

/// Petit cache de lots réhydratés, pour que faire défiler une page d'écran ne
/// relise pas le même lot une fois par cellule.
///
/// Ordre d'usage, pas horodatage : la file est parcourue à chaque accès, ce qui
/// est le moins cher pour une poignée d'entrées et évite un compteur global.
#[derive(Debug)]
pub(crate) struct SpillCache {
    entries: VecDeque<(usize, RecordBatch)>,
    capacity: usize,
}

impl SpillCache {
    /// Cache d'au plus `capacity` lots. Une capacité nulle désactive le cache.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Rend le lot `index` s'il est déjà réhydraté, en le marquant comme le
    /// plus récemment utilisé.
    pub(crate) fn get(&mut self, index: usize) -> Option<RecordBatch> {
        let position = self.entries.iter().position(|(i, _)| *i == index)?;
        let entree = self.entries.remove(position)?;
        let lot = entree.1.clone();
        self.entries.push_back(entree);
        Some(lot)
    }

    /// Enregistre un lot réhydraté, en évinçant le plus ancien si besoin.
    pub(crate) fn insert(&mut self, index: usize, batch: RecordBatch) {
        if self.capacity == 0 {
            return;
        }
        if self.entries.iter().any(|(i, _)| *i == index) {
            return;
        }
        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((index, batch));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("nom", DataType::Utf8, true),
        ]))
    }

    fn lot(base: i32, lignes: usize) -> RecordBatch {
        let ids: Vec<i32> = (0..lignes)
            .map(|i| base.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        let noms: Vec<Option<String>> = ids.iter().map(|i| Some(format!("l{i}"))).collect();
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(noms)),
            ],
        )
        .expect("les colonnes correspondent au schéma construit juste au-dessus")
    }

    #[test]
    fn un_lot_ecrit_se_relit_a_l_identique() {
        let fichier = SpillFile::create().expect("le répertoire temporaire doit être accessible");
        let original = lot(0, 128);
        let reference = fichier
            .append(&schema(), &original)
            .expect("écriture du lot");
        let relu = fichier.read(reference).expect("relecture du lot");
        assert_eq!(relu, original);
    }

    /// Le cas qui casse une implémentation naïve : plusieurs lots dans le même
    /// fichier, relus dans le désordre.
    #[test]
    fn les_lots_se_relisent_dans_le_desordre() {
        let fichier = SpillFile::create().expect("le répertoire temporaire doit être accessible");
        let schema = schema();

        let lots: Vec<RecordBatch> = (0..5_usize)
            .map(|i| lot(i32::try_from(i).unwrap_or(0) * 100, 32 + i))
            .collect();
        let references: Vec<SpillRef> = lots
            .iter()
            .map(|l| fichier.append(&schema, l).expect("écriture"))
            .collect();

        for position in [4_usize, 0, 3, 1, 2, 4] {
            let attendu = lots.get(position).expect("indice construit ci-dessus");
            let reference = *references
                .get(position)
                .expect("indice construit ci-dessus");
            assert_eq!(&fichier.read(reference).expect("relecture"), attendu);
        }
    }

    /// Écrire pendant qu'on relit : c'est le scénario réel — la requête coule
    /// encore, l'utilisateur fait défiler.
    #[test]
    fn une_ecriture_ne_perturbe_pas_les_relectures_precedentes() {
        let fichier = SpillFile::create().expect("le répertoire temporaire doit être accessible");
        let schema = schema();

        let premier = lot(0, 16);
        let r1 = fichier.append(&schema, &premier).expect("écriture");
        assert_eq!(fichier.read(r1).expect("relecture"), premier);

        let second = lot(1_000, 64);
        let r2 = fichier.append(&schema, &second).expect("écriture");

        assert_eq!(fichier.read(r1).expect("relecture"), premier);
        assert_eq!(fichier.read(r2).expect("relecture"), second);
        assert!(r2.byte_len() > r1.byte_len(), "un lot plus gros pèse plus");
    }

    #[test]
    fn le_cache_evince_le_plus_ancien() {
        let mut cache = SpillCache::new(2);
        cache.insert(0, lot(0, 1));
        cache.insert(1, lot(1, 1));
        cache.insert(2, lot(2, 1));

        assert!(cache.get(0).is_none(), "le plus ancien doit être évincé");
        assert!(cache.get(1).is_some());
        assert!(cache.get(2).is_some());
    }

    /// Un accès rafraîchit l'entrée : sinon un défilement qui alterne entre deux
    /// lots frontaliers évince en boucle celui dont il a besoin.
    #[test]
    fn un_acces_protege_de_l_eviction() {
        let mut cache = SpillCache::new(2);
        cache.insert(0, lot(0, 1));
        cache.insert(1, lot(1, 1));
        assert!(cache.get(0).is_some());
        cache.insert(2, lot(2, 1));

        assert!(cache.get(0).is_some(), "l'entrée rafraîchie doit survivre");
        assert!(cache.get(1).is_none());
    }

    #[test]
    fn un_cache_de_capacite_nulle_ne_retient_rien() {
        let mut cache = SpillCache::new(0);
        cache.insert(0, lot(0, 1));
        assert!(cache.get(0).is_none());
    }
}
