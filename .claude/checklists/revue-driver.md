# Revue d'un driver

À passer intégralement avant de considérer un driver comme livré. Le contrat
fait autorité : `docs/DRIVER-CONTRACT.md`.

Une case non cochée n'est pas un détail à traiter plus tard : chaque point ici
correspond à une panne qui ne se verra pas en test.

## Découpage

- [ ] C'est bien un **protocole** nouveau, et non un produit parlant un protocole
      déjà implémenté (Redshift ≡ PostgreSQL, MariaDB ≡ MySQL, OpenSearch ≡
      Elasticsearch)
- [ ] La crate ne dépend que d'`oxyn-core`, `oxyn-driver`, `oxyn-data` et `oxyn-catalog`
- [ ] Aucune dépendance vers un autre driver

## Robustesse

- [ ] Aucun `unwrap`, `expect`, `panic!`, indexation de tranche ni `as` débordant
      sur un chemin atteignable depuis une réponse serveur
- [ ] Testé contre : type inconnu, `NULL` sur colonne `NOT NULL`, entier hors
      bornes, encodage invalide, **réponse tronquée en plein flux**
- [ ] Testé contre un nom d'objet contenant guillemet, point-virgule, ou du texte
      imitant une consigne

## Résultats

- [ ] Produit des `RecordBatch`, jamais une représentation en lignes
- [ ] Le lot est borné **en octets**, pas en nombre de lignes
- [ ] **Test de flux sur un volume qui ne tiendrait pas en mémoire**, avec une
      borne sur la mémoire du processus — sans borne, le test passe par accident
- [ ] Si la source est sans schéma : l'inférence par échantillonnage est déclarée
      comme telle jusqu'à l'interface, et un champ hors échantillon produit une
      erreur explicite, jamais une perte silencieuse

## Annulation

- [ ] **Test prouvant l'arrêt côté serveur**, vérifié dans la vue des processus
      du SGBD — pas au retour de la fonction
- [ ] Si l'annulation côté serveur est impossible, elle est **déclarée absente**
      dans les capacités, et non simulée

## Erreurs

- [ ] Trois classes distinctes : transitoire, permanente, **ambiguë**
- [ ] La classe est une **donnée**, pas une déduction faite à partir du message
- [ ] Un délai dépassé pendant une écriture est classé **ambigu**, jamais
      transitoire
- [ ] Le driver ne retente jamais de lui-même

## Capacités

- [ ] Évaluées **par session**, pas par driver
- [ ] Rien n'est simulé : ne pas savoir faire est déclaré
- [ ] `QueryLanguage` explicite

## Types

- [ ] Table de correspondance **dans les deux sens**
- [ ] Pertes documentées, notamment : `NUMERIC` de précision arbitraire, entiers
      au-delà de 2^53, types spatiaux, types propriétaires
- [ ] Aucun fuseau attribué à un `timestamp` qui n'en a pas
- [ ] Type inconnu rendu en octets bruts **avec son identifiant de type**

## Sécurité

- [ ] Aucune valeur liée ni identifiant de connexion dans un journal
- [ ] Aucun identifiant concaténé dans du SQL composé par Oxyn
- [ ] Aucun `SET`/`USE` modifiant l'état de session sans le déclarer
- [ ] Aucune lecture de variable d'environnement, aucune écriture de fichier

## Porte de sortie

- [ ] `make qualite` passe
- [ ] `relecteur-frontiere` et `relecteur-invariants` n'ont rien de bloquant
- [ ] `docs/RESEARCH-NOTES.md` à jour si une dépendance a été ajoutée
