//! Key names are data: below the tier that carries values, only patterns leave.
//!
//! A Redis key is a relation of kind
//! [`RelationKind::KeyPattern`], and the Redis driver lists the **exact** keys
//! it finds ("an exact key is the pattern that matches only itself"). A key
//! name routinely embeds the data itself — `user:<email>`, `session:<token>` —
//! which is not what the `Metadata` tier promises: it accepts names of
//! structure, and a column name is a name of structure, where a key name often
//! is a value ([ADR-0061](../../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md),
//! [AI-PROVIDERS](../../../../docs/AI-PROVIDERS.md)).
//!
//! Under a tier that does not allow row values
//! ([`PrivacyTier::allows_row_values`]), the gateway therefore never writes a
//! key name. It writes **patterns**: the prefix up to the first separator,
//! followed by `*`, with the number of keys that share it. Exact names leave
//! only under [`PrivacyTier::Sampled`], which already carries values.
//!
//! A prefix is itself a piece of a key name, so it is sent only when it looks
//! like a namespace and not like an identifier: short, made of letters, digits,
//! `_` and `-`, shared by at least [`MIN_KEYS_PER_PATTERN`] keys, and not
//! carrying a run of digits. Every other key is counted without being
//! described. The rule errs towards withholding: a namespace that falls under
//! it is still counted, and the user can name the key in the question.

use std::collections::BTreeMap;

use oxyn_catalog::CatalogCache;
use oxyn_catalog::model::RelationKind;

use super::Naming;
use crate::privacy::PrivacyTier;

/// Characters that end a key's namespace, by the usual Redis conventions.
const SEPARATORS: [char; 5] = [':', '/', '|', '.', '#'];

/// The longest prefix sent as a pattern, in characters.
const MAX_PREFIX_CHARS: usize = 32;

/// The most digits a prefix may hold: more reads as an identifier
/// (`session9f8a7b6c`, `user123456`), not as a namespace.
const MAX_PREFIX_DIGITS: usize = 4;

/// A prefix shared by fewer keys is the name of one entity, not of a family.
const MIN_KEYS_PER_PATTERN: usize = 2;

/// The most patterns written for the whole connection; the rest is counted.
const MAX_PATTERNS: usize = 24;

/// Does the tier forbid writing key names at all?
fn tier_withholds(tier: PrivacyTier) -> bool {
    withholds_name(tier, RelationKind::KeyPattern)
}

/// Does the tier forbid writing the name of a relation of this kind?
pub(super) fn withholds_name(tier: PrivacyTier, kind: RelationKind) -> bool {
    kind == RelationKind::KeyPattern && !tier.allows_row_values()
}

/// The prefix of a key, up to and including its first separator, if it can be
/// sent.
pub(super) fn namespace_of(key: &str) -> Option<&str> {
    let end = key.find(SEPARATORS)?;
    let prefix = key.get(..end)?;
    let acceptable = !prefix.is_empty()
        && prefix.chars().count() <= MAX_PREFIX_CHARS
        && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && prefix.chars().filter(char::is_ascii_digit).count() <= MAX_PREFIX_DIGITS;
    // The separator stays: `session:*` and `session/*` are different patterns.
    acceptable.then(|| key.get(..=end)).flatten()
}

/// Writes the patterns of every key the cache holds, or nothing when it holds
/// none.
///
/// Counted over the whole cache and not over the selected relations: the
/// patterns are the connection's key structure, and the selection (a question
/// matched against key names) only ranks what is described individually, which
/// this tier does not do for keys.
pub(super) fn render(cache: &CatalogCache, tier: PrivacyTier, naming: Naming) -> String {
    // (parent path, prefix) -> keys; `parents` counts every key per parent.
    if !tier_withholds(tier) {
        return String::new();
    }
    let mut families: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut parents: BTreeMap<String, usize> = BTreeMap::new();

    for (summary, _) in cache.iter_relations() {
        if summary.kind != RelationKind::KeyPattern {
            continue;
        }
        let path = summary.path();
        let parent = path
            .parent()
            .map(|parent| naming.path(&parent))
            .unwrap_or_default();
        *parents.entry(parent.clone()).or_default() += 1;
        if let Some(prefix) = path.relation().and_then(namespace_of) {
            *families.entry((parent, prefix.to_owned())).or_default() += 1;
        }
    }
    if parents.is_empty() {
        return String::new();
    }

    // Biggest families first; the name breaks ties so two builds agree.
    let mut ranked: Vec<_> = families
        .into_iter()
        .filter(|(_, keys)| *keys >= MIN_KEYS_PER_PATTERN)
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(MAX_PATTERNS);

    let mut described: BTreeMap<&str, usize> = BTreeMap::new();
    let mut lines = String::new();
    for ((parent, prefix), keys) in &ranked {
        let pattern = naming.name(&format!("{prefix}*"));
        let subject = if parent.is_empty() {
            pattern
        } else {
            format!("{parent}.{pattern}")
        };
        lines.push_str(&format!("  key_pattern {subject} — {keys} keys\n"));
        *described.entry(parent.as_str()).or_default() += keys;
    }
    for (parent, total) in &parents {
        let rest = total.saturating_sub(described.get(parent.as_str()).copied().unwrap_or(0));
        if rest == 0 {
            continue;
        }
        let place = if parent.is_empty() {
            String::new()
        } else {
            format!(" in {parent}")
        };
        lines.push_str(&format!("  {rest} other keys{place}, no pattern shared\n"));
    }

    format!(
        "key names are not sent under privacy tier `{tier}`: only the prefix up to the first \
         separator and a count (a key name often holds the data itself); exact key names leave \
         only under `sampled`. Ask the user for a key name instead of guessing one.\n{lines}\n"
    )
}
