---
name: piege-reqwest-delai-de-connexion
description: reqwest marque un délai de connexion à la fois is_connect() et is_timeout() — tester is_timeout() d'abord classe ambigu ce qui n'est jamais parti
metadata:
  type: reference
---

Dans reqwest 0.13.4, `connect_timeout` est appliqué **dans le connecteur**
(`src/connect.rs`, `crate::error::TimedOut`), et hyper-util enveloppe toute erreur
de connecteur en `ErrorKind::Connect`. Une erreur de délai de connexion répond donc
`is_connect() == true` **et** `is_timeout() == true`. Un délai `.timeout()` global
expiré après l'envoi répond `is_timeout()` seul.

**Why:** l'ordre des tests décide de la famille d'erreur (I-13). `is_timeout()` en
premier classe « ambigu, peut-être facturé » une requête qui n'a jamais quitté la
machine ; l'inverse rend rejouable une requête partie.

**How to apply:** toujours `is_connect()` avant `is_timeout()`. Re-vérifier dans les
sources du registre (`~/.cargo/registry/src/*/reqwest-*/src/error.rs`, `connect.rs`)
à chaque montée de reqwest. Un délai de connexion ne se provoque pas sans réseau
(il faut un SYN sans réponse) : un test local ne couvre que le refus et le délai de
réponse. `#[tokio::test]` compile dans `oxyn-llm` grâce aux fonctionnalités tokio
tirées par reqwest/hyper-util, pas par son propre manifeste.
