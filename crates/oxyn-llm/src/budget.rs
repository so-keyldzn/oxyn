//! Ce qu'une génération peut accumuler, au total.
//!
//! Le décodeur SSE borne la trame **en cours** ; rien ne bornait leur somme.
//! Une suite de petites trames, chacune valide et sous le plafond, faisait
//! grossir sans fin le texte, les arguments d'un appel d'outil ou le nombre de
//! blocs ouverts — jusqu'à ce que le système tue le processus, sans trace.
//!
//! Un [`GenerationBudget`] se tient pour **une** génération. Chaque octet
//! accumulé ou émis y est compté avant de l'être, et chaque bloc ou appel
//! d'outil ouvert aussi. Le premier dépassement arrête la génération : le
//! décodeur le signale par une erreur qui nomme la limite, jette ce qui n'était
//! pas clos — un appel d'outil coupé n'est pas une proposition d'action — et
//! termine le flux en [`StopReason::Interrupted`](crate::types::StopReason) :
//! le fournisseur a peut-être continué, et facturé, ce qu'on a cessé de lire
//! (I-13).
//!
//! Les limites sont des choix d'Oxyn, pas des valeurs de fournisseur : elles
//! bornent la mémoire, elles ne cherchent pas à coller au plus grand modèle du
//! moment. Chacune est placée loin au-dessus de ce qu'un assistant SQL produit
//! pour être lu.

use std::fmt;

/// Octets de contenu cumulés sur une génération : texte, refus,
/// raisonnement, noms et arguments d'outils, identifiants.
///
/// Même ordre de grandeur que la borne d'une trame SSE
/// (`sse::DEFAULT_BUFFER_LIMIT`) : une réponse de huit mébioctets se lit en
/// plusieurs heures, et au-delà la génération ne sert plus personne mais peut
/// encore épuiser la mémoire.
pub const MAX_GENERATION_BYTES: usize = 8 * 1024 * 1024;

/// Octets d'arguments d'**un** appel d'outil.
///
/// Les arguments d'un outil d'Oxyn sont une instruction SQL ou une adresse de
/// catalogue. Un mébioctet de SQL proposé par un modèle n'est plus relisible
/// par l'utilisateur qui doit l'approuver ; c'est un flux qui dérape.
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 1024 * 1024;

/// Octets du nom d'un outil.
///
/// Les noms valables sont ceux du registre d'Oxyn, quelques dizaines
/// d'octets. Un nom plus long ne désignera jamais un outil connu : le laisser
/// grossir — certains serveurs le fragmentent — n'aurait aucun usage.
pub const MAX_TOOL_NAME_BYTES: usize = 256;

/// Appels d'outils ouverts dans une génération.
///
/// Chaque appel devient une `Command` que le `PolicyGate` juge, et que
/// l'utilisateur peut avoir à approuver une par une. Au-delà de quelques
/// dizaines par tour, ce n'est plus une conversation mais une rafale.
pub const MAX_TOOL_CALLS: usize = 64;

/// Blocs de contenu ouverts dans une génération — texte, raisonnement,
/// outil, types inconnus compris.
///
/// Chaque bloc ouvert porte un état jusqu'à sa fermeture ; un serveur qui en
/// ouvre sans jamais les fermer fait croître cet état sans rien émettre.
/// Le plafond est au-dessus de [`MAX_TOOL_CALLS`], puisqu'un appel d'outil
/// est un bloc.
pub const MAX_CONTENT_BLOCKS: usize = 256;

/// Une limite atteinte. Le message la nomme et donne sa valeur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BudgetExceeded {
    /// [`MAX_GENERATION_BYTES`].
    Generation,
    /// [`MAX_TOOL_ARGUMENTS_BYTES`], pour l'appel d'index donné.
    ToolArguments {
        /// L'index de l'appel dans la génération.
        index: u32,
    },
    /// [`MAX_TOOL_NAME_BYTES`], pour l'appel d'index donné.
    ToolName {
        /// L'index de l'appel dans la génération.
        index: u32,
    },
    /// [`MAX_TOOL_CALLS`].
    ToolCalls,
    /// [`MAX_CONTENT_BLOCKS`].
    ContentBlocks,
}

impl fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arret = "Oxyn stopped reading the answer";
        match self {
            Self::Generation => write!(
                f,
                "the answer exceeded {MAX_GENERATION_BYTES} bytes; {arret}"
            ),
            Self::ToolArguments { index } => write!(
                f,
                "the arguments of tool call #{index} exceeded {MAX_TOOL_ARGUMENTS_BYTES} bytes; {arret}"
            ),
            Self::ToolName { index } => write!(
                f,
                "the name of tool call #{index} exceeded {MAX_TOOL_NAME_BYTES} bytes; {arret}"
            ),
            Self::ToolCalls => write!(
                f,
                "the answer opened more than {MAX_TOOL_CALLS} tool calls; {arret}"
            ),
            Self::ContentBlocks => write!(
                f,
                "the answer opened more than {MAX_CONTENT_BLOCKS} content blocks; {arret}"
            ),
        }
    }
}

impl std::error::Error for BudgetExceeded {}

/// Le compte d'une génération.
///
/// Rien n'est jamais « rendu » au budget : un bloc fermé a déjà coûté sa
/// mémoire à un moment, et le texte émis est accumulé plus loin par
/// l'appelant.
#[derive(Debug, Default, Clone)]
pub struct GenerationBudget {
    bytes: usize,
    tool_calls: usize,
    blocks: usize,
}

impl GenerationBudget {
    /// Un budget neuf, pour une génération.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Compte `len` octets de contenu.
    ///
    /// # Erreurs
    /// [`BudgetExceeded::Generation`] si le cumul dépasserait
    /// [`MAX_GENERATION_BYTES`] ; rien n'est alors compté.
    pub fn charge(&mut self, len: usize) -> Result<(), BudgetExceeded> {
        let total = self.bytes.saturating_add(len);
        if total > MAX_GENERATION_BYTES {
            return Err(BudgetExceeded::Generation);
        }
        self.bytes = total;
        Ok(())
    }

    /// Appels d'outils déjà ouverts : l'index du prochain.
    #[must_use]
    pub const fn tool_calls(&self) -> usize {
        self.tool_calls
    }

    /// Compte un bloc de contenu ouvert.
    ///
    /// # Erreurs
    /// [`BudgetExceeded::ContentBlocks`] au-delà de [`MAX_CONTENT_BLOCKS`].
    pub fn open_block(&mut self) -> Result<(), BudgetExceeded> {
        if self.blocks >= MAX_CONTENT_BLOCKS {
            return Err(BudgetExceeded::ContentBlocks);
        }
        self.blocks += 1;
        Ok(())
    }

    /// Compte un appel d'outil ouvert, qui est aussi un bloc.
    ///
    /// # Erreurs
    /// [`BudgetExceeded::ToolCalls`] au-delà de [`MAX_TOOL_CALLS`], ou le refus
    /// de [`open_block`](Self::open_block).
    pub fn open_tool_call(&mut self) -> Result<(), BudgetExceeded> {
        if self.tool_calls >= MAX_TOOL_CALLS {
            return Err(BudgetExceeded::ToolCalls);
        }
        self.open_block()?;
        self.tool_calls += 1;
        Ok(())
    }

    /// Compte un fragment d'arguments de l'appel `index`, dont les arguments
    /// font déjà `current` octets.
    ///
    /// # Erreurs
    /// [`BudgetExceeded::ToolArguments`] si l'appel dépasserait
    /// [`MAX_TOOL_ARGUMENTS_BYTES`], ou le refus de [`charge`](Self::charge).
    pub fn charge_tool_arguments(
        &mut self,
        index: u32,
        current: usize,
        fragment: usize,
    ) -> Result<(), BudgetExceeded> {
        if current.saturating_add(fragment) > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(BudgetExceeded::ToolArguments { index });
        }
        self.charge(fragment)
    }

    /// Vérifie qu'un appel **complet** tient dans [`MAX_TOOL_ARGUMENTS_BYTES`],
    /// sans le compter dans le cumul.
    ///
    /// Pour l'appelant qui reçoit l'appel entier après ses fragments : ceux-ci
    /// sont déjà comptés, les recompter réduirait le budget de moitié. Pour un
    /// fournisseur qui livre l'appel d'un bloc, c'est la seule vérification.
    ///
    /// # Erreurs
    /// [`BudgetExceeded::ToolArguments`] au-delà de la limite.
    pub fn check_tool_arguments(&self, index: u32, len: usize) -> Result<(), BudgetExceeded> {
        if len > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(BudgetExceeded::ToolArguments { index });
        }
        Ok(())
    }

    /// Compte un fragment du nom de l'appel `index`, dont le nom fait déjà
    /// `current` octets.
    ///
    /// # Erreurs
    /// [`BudgetExceeded::ToolName`] si le nom dépasserait
    /// [`MAX_TOOL_NAME_BYTES`], ou le refus de [`charge`](Self::charge).
    pub fn charge_tool_name(
        &mut self,
        index: u32,
        current: usize,
        fragment: usize,
    ) -> Result<(), BudgetExceeded> {
        if current.saturating_add(fragment) > MAX_TOOL_NAME_BYTES {
            return Err(BudgetExceeded::ToolName { index });
        }
        self.charge(fragment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generation_budget_refuses_the_byte_past_its_limit_and_counts_nothing() {
        let mut budget = GenerationBudget::new();
        budget
            .charge(MAX_GENERATION_BYTES - 1)
            .expect("under the limit");
        budget.charge(1).expect("exactly at the limit");
        assert_eq!(budget.charge(1), Err(BudgetExceeded::Generation));
        assert_eq!(budget.charge(usize::MAX), Err(BudgetExceeded::Generation));
    }

    #[test]
    fn a_tool_call_is_also_a_block() {
        let mut budget = GenerationBudget::new();
        for _ in 0..MAX_TOOL_CALLS {
            budget.open_tool_call().expect("under the call limit");
        }
        assert_eq!(budget.open_tool_call(), Err(BudgetExceeded::ToolCalls));
        for _ in MAX_TOOL_CALLS..MAX_CONTENT_BLOCKS {
            budget.open_block().expect("under the block limit");
        }
        assert_eq!(budget.open_block(), Err(BudgetExceeded::ContentBlocks));
    }

    #[test]
    fn each_message_names_its_limit() {
        for (limite, valeur) in [
            (BudgetExceeded::Generation, MAX_GENERATION_BYTES),
            (
                BudgetExceeded::ToolArguments { index: 3 },
                MAX_TOOL_ARGUMENTS_BYTES,
            ),
            (BudgetExceeded::ToolName { index: 3 }, MAX_TOOL_NAME_BYTES),
            (BudgetExceeded::ToolCalls, MAX_TOOL_CALLS),
            (BudgetExceeded::ContentBlocks, MAX_CONTENT_BLOCKS),
        ] {
            let rendu = limite.to_string();
            assert!(rendu.contains(&valeur.to_string()), "{rendu}");
            assert!(rendu.contains("stopped"), "{rendu}");
        }
    }
}
