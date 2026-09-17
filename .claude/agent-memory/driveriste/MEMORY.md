# Mémoire — driveriste

- [Cluster PostgreSQL jetable](outil-cluster-postgres-jetable.md) — le recréer (initdb, `-k ''`), rôle `oxyn_test`, les tests rouges qui ne sont pas des régressions
- [Annulation mal ciblée, fenêtre reproductible](piege-annulation-fenetre-deterministe.md) — drapeau `sqlite3_interrupt` par connexion, pid réutilisé par le bassin, verrous consultatifs et `ClientWrite` en test
- [Bassin PostgreSQL et cache sqlx](piege-bassin-postgres-et-cache-sqlx.md) — forcer plusieurs connexions dans un test, connexion abandonnée = connexion fermée, préparation servie par le cache
- [SQL composé autour du texte de l'utilisateur](piege-sql-compose-autour-du-texte-utilisateur.md) — saut de ligne contre le `--`, parenthèses contre le `/*` non fermé, et le `;` qui s'exécute vraiment en SQLite
