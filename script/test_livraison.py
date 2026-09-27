#!/usr/bin/env python3
"""Tests of `script/livraison`, with a simulated `gh`.

The fake `gh` keeps a release's state in a JSON file and logs every call: a
test prepares the state, runs the script, then looks at what was called and
what remains. It reproduces the documented behavior of
`gh release upload --clobber` — the old file is deleted before the upload — so
that the option coming back shows here.

Called by `make socle`, hence by `make qualite`.
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
        # A human publication during the builds: the release flips after a
        # given number of reads.
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
    sortir(2, "unexpected verb")
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

    # --- #30: the tag carries the version of tauri.conf.json ---------------

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
                self.assertIn("expected v0.0.1", sortie.stderr)
                self.assertEqual(self.etat()["appels"], [], "gh called despite a wrong tag")

    def test_preversion_configuree_refuse_le_tag_sans_suffixe(self) -> None:
        self.version("0.1.0-rc.1")
        sortie = self.lancer("brouillon", "v0.1.0", DEPOT)
        self.assertEqual(sortie.returncode, 1)
        self.assertEqual(self.etat()["appels"], [])

    def test_tag_incoherent_n_envoie_aucun_paquet(self) -> None:
        self.ecrire_etat(release={"isDraft": True, "assets": []})
        sortie = self.lancer("deposer", "v9.9.9", DEPOT, self.paquet("Oxyn_0.0.1.dmg"))
        self.assertEqual(sortie.returncode, 1)
        self.assertEqual(self.etat()["appels"], [])

    def test_version_renvoyant_a_un_package_json_est_refusee(self) -> None:
        self.version("../../apps/desktop/package.json")
        sortie = self.lancer("brouillon", "v0.0.1", DEPOT)
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("not supported here", sortie.stderr)

    # --- #20: nothing touches a published release -------------------------

    def test_relance_sur_une_release_publiee_echoue_au_premier_job(self) -> None:
        actifs = [{"name": "Oxyn_0.0.1.dmg"}]
        self.ecrire_etat(release={"isDraft": False, "assets": actifs})
        sortie = self.lancer("brouillon", "v0.0.1", DEPOT)
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("is published", sortie.stderr)
        self.assertEqual(self.verbes(), ["view", "view"])
        self.assertEqual(self.etat()["release"]["assets"], actifs)

    def test_relance_sur_une_release_publiee_n_altere_aucun_actif(self) -> None:
        actifs = [{"name": "Oxyn_0.0.1.dmg"}]
        self.ecrire_etat(release={"isDraft": False, "assets": actifs})
        # A package absent from the public release: only the status stops it.
        sortie = self.lancer("deposer", "v0.0.1", DEPOT, self.paquet("oxyn_0.0.1.deb"))
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("is published", sortie.stderr)
        self.assertNotIn("upload", self.verbes())
        self.assertEqual(self.etat()["release"]["assets"], actifs)

    def test_publication_pendant_le_build_empeche_le_depot(self) -> None:
        self.assertEqual(self.lancer("brouillon", "v0.0.1", DEPOT).returncode, 0)
        etat = self.etat()
        etat["release"]["isDraft"] = False
        etat["appels"] = []
        self.etat_chemin.write_text(json.dumps(etat), encoding="utf-8")

        sortie = self.lancer("deposer", "v0.0.1", DEPOT, self.paquet("Oxyn_0.0.1.dmg"))
        self.assertEqual(sortie.returncode, 1)
        self.assertNotIn("upload", self.verbes())
        self.assertEqual(self.etat()["release"]["assets"], [])

    def test_statut_relu_avant_chaque_fichier(self) -> None:
        # Published after the two reads around the first upload: the first
        # package goes, the second is refused before any upload.
        self.ecrire_etat(release={"isDraft": True, "assets": []}, publier_apres=2)
        sortie = self.lancer(
            "deposer", "v0.0.1", DEPOT, self.paquet("oxyn_0.0.1.deb"), self.paquet("oxyn-0.0.1.rpm")
        )
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("is published", sortie.stderr)
        self.assertEqual(self.verbes(), ["view", "upload", "view", "view"])
        self.assertEqual(self.etat()["release"]["assets"], [{"name": "oxyn_0.0.1.deb"}])

    def test_publication_pendant_l_envoi_arrete_le_depot(self) -> None:
        # Published between the read and the end of the first package's upload.
        self.ecrire_etat(release={"isDraft": True, "assets": []}, publier_apres=1)
        sortie = self.lancer(
            "deposer", "v0.0.1", DEPOT, self.paquet("oxyn_0.0.1.deb"), self.paquet("oxyn-0.0.1.rpm")
        )
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("while uploading oxyn_0.0.1.deb", sortie.stderr)
        self.assertEqual(self.verbes(), ["view", "upload", "view"])

    def test_depot_dans_un_brouillon(self) -> None:
        self.ecrire_etat(release={"isDraft": True, "assets": []})
        sortie = self.lancer(
            "deposer", "v0.0.1", DEPOT, self.paquet("oxyn_0.0.1.deb"), self.paquet("oxyn-0.0.1.rpm")
        )
        self.assertEqual(sortie.returncode, 0, sortie.stderr)
        noms = [a["name"] for a in self.etat()["release"]["assets"]]
        self.assertEqual(noms, ["oxyn_0.0.1.deb", "oxyn-0.0.1.rpm"])

    def test_aucun_envoi_ne_porte_clobber(self) -> None:
        self.ecrire_etat(release={"isDraft": True, "assets": []})
        self.lancer("deposer", "v0.0.1", DEPOT, self.paquet("Oxyn_0.0.1.dmg"))
        envois = [a for a in self.etat()["appels"] if a[1] == "upload"]
        self.assertEqual(len(envois), 1)
        self.assertNotIn("--clobber", envois[0])

    def test_fichier_deja_present_n_est_pas_remplace(self) -> None:
        actifs = [{"name": "Oxyn_0.0.1.dmg"}]
        self.ecrire_etat(release={"isDraft": True, "assets": actifs})
        sortie = self.lancer("deposer", "v0.0.1", DEPOT, self.paquet("Oxyn_0.0.1.dmg"))
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("is not replaced", sortie.stderr)
        self.assertNotIn("upload", self.verbes())
        self.assertEqual(self.etat()["release"]["assets"], actifs)

    def test_echec_d_envoi_ne_detruit_aucun_actif(self) -> None:
        actifs = [{"name": "oxyn_0.0.1.deb"}]
        self.ecrire_etat(release={"isDraft": True, "assets": actifs}, echec_envoi=True)
        sortie = self.lancer("deposer", "v0.0.1", DEPOT, self.paquet("oxyn-0.0.1.rpm"))
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("HTTP 502", sortie.stderr)
        self.assertEqual(self.etat()["release"]["assets"], actifs)

    def test_echec_de_creation_fait_echouer_le_job(self) -> None:
        self.ecrire_etat(release=None, echec_creation=True)
        sortie = self.lancer("brouillon", "v0.0.1", DEPOT)
        self.assertEqual(sortie.returncode, 1)
        self.assertIn("tag does not exist", sortie.stderr)


if __name__ == "__main__":
    unittest.main()
