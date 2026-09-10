# ADR-0014 — Séparer les brouillons, les requêtes sauvegardées et l'historique

**Statut :** accepté · **Date :** 2026-09-10

## Contexte

La planche Figma `47:7638` présente History, Saved queries et Recent results.
Les tables locales existent, mais leurs listes chargent le SQL complet et
les documents ne distinguent pas un brouillon d'une sauvegarde explicite.
Ajouter une sauvegarde automatique sans cette distinction modifierait une
requête enregistrée pendant que l'utilisateur travaille sur un brouillon.

## Décision

- La migration 5 conserve les documents existants comme requêtes sauvegardées.
  Elle ajoute l'état ouvert/supprimé, la révision du brouillon, la révision de
  sauvegarde et les texte/titre explicitement sauvegardés. Le format reste
  constitué de colonnes SQLite lisibles, sans type GPUI.
- Les révisions de brouillon et de sauvegarde sont indépendantes. Un brouillon
  récent ne doit pas annuler une sauvegarde explicite antérieure qui termine
  plus tard ; cette dernière ne remplace pas le brouillon récent.
- La fermeture avec abandon rétablit la copie sauvegardée, ou vide un brouillon
  sans copie sauvegardée. Un marqueur de révision empêche une ancienne écriture
  de rouvrir ou de recréer le document. La suppression explicite retire aussi
  les textes, tout en conservant ce marqueur minimal.
- Les listes passent par le bus et restent paginées : au plus 200 entrées par
  page, résumés SQL limités à 256 caractères. Un document ou une entrée
  sélectionnée est lu séparément. Le SQL éditable est borné à 1 Mio et les titres
  à 256 octets ; un dépassement produit une erreur explicite, jamais une coupe
  silencieuse du texte ouvert ou sauvegardé.
- L'historique conserve sa portée existante : les connexions locales de
  l'utilisateur, y compris celles supprimées. Il n'est pas présenté comme une
  liste appartenant nécessairement au seul workspace actif. Les filtres de
  connexion, texte, date et statut sont explicites. La pagination suit l'ordre
  d'enregistrement décroissant, avec une borne d'identifiant stable.
- Une entrée d'historique peut référencer un résultat encore retenu. Cette
  référence ne restaure aucune session et n'exécute aucune requête. Une
  référence expirée produit une indisponibilité explicite.
- Ouvrir un document ou du SQL historique crée un contexte d'édition sans
  exécution. Une copie vers une autre connexion est annoncée et conserve
  l'original. L'historique d'une écriture ambiguë offre l'inspection, pas une
  action de rejeu.

## Conséquences

- **+** Les brouillons récupérables ne modifient pas la bibliothèque sauvegardée.
- **+** Les écritures asynchrones ne peuvent pas rétablir un document fermé.
- **+** Les listes ne matérialisent pas tout le SQL local en mémoire.
- **−** Chaque document conserve deux états de texte et deux révisions.
- **−** Les petits marqueurs de suppression restent dans le store pour protéger
  des réponses tardives ; ils ne figurent pas dans les listes utilisateur.
- **−** Les fichiers SQL dépassant la borne d'édition nécessitent un traitement
  distinct et ne sont pas ouverts partiellement sous leur nom d'origine.

**Coût de sortie :** migrer les colonnes de documents et les commandes de
sauvegarde/fermeture. Les drivers et le contenu SQL restent indépendants.

**Reconsidérer si** l'édition collaborative exige une fusion de contenu, si les
gros scripts deviennent un usage courant, ou si la rétention des marqueurs
nécessite une politique d'expiration liée aux sessions locales.

## Alternatives écartées

| Alternative | Raison du rejet |
|---|---|
| Un seul texte pour brouillon et sauvegarde | Toute frappe modifie la copie enregistrée |
| Supprimer immédiatement la ligne d'un document fermé | Une réponse tardive peut la recréer |
| Charger toutes les requêtes dans la liste | Mémoire proportionnelle à tout l'historique |
| Rejouer pour rouvrir un résultat | Change les données observées et peut répéter une écriture |
