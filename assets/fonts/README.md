# Geist pour l’interface Oxyn

Geist Regular, Medium et SemiBold sont les trois graisses utilisées par le
design Figma. Fichiers TTF téléchargés sans transformation le 2026-09-07 depuis
[vercel/geist-font](https://github.com/vercel/geist-font/tree/10dc7658f13c38a474cde201bb09a4617267545b/fonts/Geist/ttf).
Le commit et les SHA-256 sont conservés dans [provenance.json](provenance.json).

Les notices amont [OFL.txt](OFL.txt) et [LICENSE.txt](LICENSE.txt) sont conservées
à côté des polices : SIL Open Font License 1.1.

Au démarrage, appeler `cx.text_system().add_fonts(UiAssets::fonts())` avant
l’ouverture des fenêtres. Ces octets sont inclus à la compilation et ne
dépendent d’aucune police installée sur la machine. `Typography::ui_family`
désigne `Geist`. Si l’enregistrement échoue, le démarrage doit signaler l’erreur ;
le moteur GPUI assure alors son repli de plateforme, dont les métriques peuvent
différer du design. La famille monospace existante `Menlo` reste utilisée pour
les surfaces techniques.
