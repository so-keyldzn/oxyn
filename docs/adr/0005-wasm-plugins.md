# ADR-0005 — Plugins WebAssembly, pas de bibliothèques natives

**Statut :** proposé · **Date :** 2026-09-05

## Contexte
« Extensible through plugins » face à « Privacy first ». Un plugin natif (dylib) peut
faire crasher le workspace, lire le trousseau et exfiltrer des identifiants.

## Décision
Hôte **wasmtime** avec le Component Model et des interfaces WIT. Trois surfaces :
drivers (`oxyn:driver`), agents (déclaratifs, sans code), formats d'export et
visualisations. Permissions déclarées au manifeste et approuvées à l'installation.

## Conséquences
* **+** Un plugin défaillant ne peut ni crasher ni exfiltrer.
* **+** Accès réseau accordé hôte par hôte, port par port.
* **−** Surcoût d'exécution et de sérialisation aux frontières (acceptable : les drivers
  sont dominés par la latence réseau).
* **−** Écrire un driver en plugin est plus contraignant qu'en crate interne — d'où le
  report en phase 4, une fois les traits stabilisés par 6+ implémentations natives.
