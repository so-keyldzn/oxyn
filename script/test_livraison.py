#!/usr/bin/env python3
"""Tests de `script/livraison`, avec un `gh` simulé.

Le faux `gh` tient l'état d'une release dans un fichier JSON et journalise
chaque appel : un test prépare l'état, lance le script, puis regarde ce qui a
été appelé et ce qui reste. Il reproduit le comportement documenté de
`gh release upload --clobber` — l'ancien fichier est supprimé avant l'envoi —
pour qu'un retour de l'option se voie ici.

Appelé par `make socle`, donc par `make qualite`.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "livraison"
DEPOT = "exemple/oxyn"

FAUX_GH = textwrap.dedent(
    """\
    import json, os, sys
    etat_chemin = os.environ["FAUX_GH_ETAT"]
    with open(etat_chemin, encoding="utf-8") as f:
        etat = json.load(f)
    etat["appels"].append(sys.argv[1:])

    def sortir(code, message=""):
        with open(etat_chemin, "w", encoding="utf-8") as f:
            json.dump(etat, f)
        if message:
            print(message, file=sys.stderr if code else sys.stdout)
        sys.exit(code)

    _, verbe, tag, *reste = sys.argv[1:]
    release = etat["release"]
    if verbe == "view":
        if release is None:
            sortir(1, "release not found")
        # Une publication humaine pendant les builds : la release bascule
        # après un nombre donné de lectures.
        etat["lectures"] += 1
        if etat["publier_apres"] is not None and etat["lectures"] > etat["publier_apres"]:
            release["isDraft"] = False
        sortir(0, json.dumps(release))
    if verbe == "create":
        if etat["echec_creation"]:
            sortir(1, "HTTP 422: tag does not exist")
        etat["release"] = {"isDraft": "--draft" in reste, "assets": [],
                           "isPrerelease": "--prerelease" in reste}
        sortir(0)
    if verbe == "upload":
        nom = os.path.basename(reste[0])
        if "--clobber" in reste:
            release["assets"] = [a for a in release["assets"] if a["name"] != nom]
        elif any(a["name"] == nom for a in release["assets"]):
            sortir(1, "asset under the same name already exists")
        if etat["echec_envoi"]:
            sortir(1, "HTTP 502: upload failed")
        release["assets"].append({"name": nom})
        sortir(0)
    sortir(2, "verbe inattendu")
    """
)


class Livraison(unittest.TestCase):
    def setUp(self) -> None:
        self._dossier = tempfile.TemporaryDirectory()
        self.dossier = Path(self._dossier.name)
        self.faux_gh = self.dossier / "gh"
        self.faux_gh.write_text(f"#!{sys.executable}\n{FAUX_GH}", encoding="utf-8")
        self.faux_gh.chmod(0o755)
        self.etat_chemin = self.dossier / "etat.json"
        self.config = self.dossier / "tauri.conf.json"
        self.version("0.0.1")
        self.ecrire_etat(release=None)

    def tearDown(self) -> None:
        self._dossier.cleanup()

    def version(self, version: str) -> None:
        self.config.write_text(json.dumps({"version": version}), encoding="utf-8")

    def ecrire_etat(self, release, publier_apres=None, echec_creation=False, echec_envoi=False):
        etat = {
            "release": release,
            "publier_apres": publier_apres,
            "echec_creation": echec_creation,
            "echec_envoi": echec_envoi,
            "lectures": 0,
            "appels": [],
        }
        self.etat_chemin.write_text(json.dumps(etat), encoding="utf-8")

    def etat(self) -> dict:
        return json.loads(self.etat_chemin.read_text(encoding="utf-8"))

    def paquet(self, nom: str) -> str:
        chemin = self.dossier / nom
        chemin.write_bytes(b"paquet")
        return str(chemin)

    def lancer(self, *args: str) -> subprocess.CompletedProcess[str]:
        env = {
            **os.environ,
            "GH": str(self.faux_gh),
            "FAUX_GH_ETAT": str(self.etat_chemin),
            "OXYN_CONFIG_TAURI": str(self.config),
        }
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args], capture_output=True, text=True, env=env, check=False
        )

    def verbes(self) -> list[str]:
        return [appel[1] for appel in self.etat()["appels"]]

    # --- #30 : le tag porte la version de tauri.conf.json -------------------

    def test_version_normale_cree_un_brouillon_sans_preversion(self) -> None:
        sortie = self.lancer("brouillon", "v0.0.1", DEPOT)
        self.assertEqual(sortie.returncode, 0, sortie.stderr)
        creation = next(a for a in self.etat()["appels"] if a[1] == "create")
        self.assertIn("--draft", creation)
        self.assertIn("--verify-tag", creation)
        self.assertNotIn("--prerelease", creation)
        self.assertTrue(self.etat()["release"]["isDraft"])

    def test_preversion_cree_un_brouillon_marque_preversion(self) -> None:
        self.version("0.1.0-rc.1")
        sortie = self.lancer("brouillon", "v0.1.0-rc.1", DEPOT)
        self.assertEqual(sortie.returncode, 0, sortie.stderr)
        self.assertTrue(self.etat()["release"]["isPrerelease"])

    def test_metadonnees_de_build_ne_font_pas_une_preversion(self) -> None:
        self.version("1.0.0+build-1")
        sortie = self.lancer("brouillon", "v1.0.0+build-1", DEPOT)
        self.assertEqual(sortie.returncode, 0, sortie.stderr)
        self.assertFalse(self.etat()["release"]["isPrerelease"])

    def test_tag_incoherent_echoue_sans_rien_creer(self) -> None:
        for tag in ("v9.9.9", "0.0.1", "v0.0.1-rc.1", "v0.0.10"):
            with self.subTest(tag=tag):
                self.ecrire_etat(release=None)
                sortie = self.lancer("brouillon", tag, DEPOT)
                self.assertEqual(sortie.returncode, 1)
                self.assertIn("attendu v0.0.1", sortie.stderr)
                self.assertEqual(self.etat()["appels"], [], "gh appelé malgré un tag faux")

    def test_preversion_configuree_refuse_le_tag_sans_suffixe(self) -> None:
        self.version("0.1.0-rc.1")
        sortie = self.lancer("brouillon", "v0.1.0", DEPOT)
        self.assertEqual(sortie.returncode, 1)
        self.assertEqual(self.etat()["appels"], [])

    def test_version_renvoyant_a_un_package_json_est_refusee(self) -> None:
        self.version("../../apps/desktop/package.json")
        sortie = self.lancer("brouillon", "v0.0.1", DEPOT)
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("non pris en charge", sortie.stderr)


if __name__ == "__main__":
    unittest.main()
