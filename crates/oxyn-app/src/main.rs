//! Oxyn — the modern database workspace.
//!
//! This binary does three things and no more: it starts logging, it assembles
//! the backend, and it opens the window. Everything else belongs to a crate
//! that can be tested without a screen
//! ([ARCHITECTURE](../../../docs/ARCHITECTURE.md#le-sens-des-dépendances)).

mod backend;
mod credentials;
mod picker;
mod recovery;
mod root;
mod workspace;

use anyhow::{Context as _, Result};
// `prelude` porte `AppContext`, d'où vient `App::new` : sans lui, la création
// de la vue racine ne compile pas, et l'erreur ne nomme pas le trait manquant.
use gpui::prelude::*;
use gpui::{Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use oxyn_ui::UiAssets;
use tracing_subscriber::EnvFilter;

use crate::backend::Backend;
use crate::root::Root;

/// Prevents shutdown callbacks from scheduling another shutdown.
#[derive(Default)]
struct ShutdownStarted;
impl gpui::Global for ShutdownStarted {}

/// The window Oxyn opens on first run.
const LARGEUR: f32 = 1280.0;
const HAUTEUR: f32 = 820.0;

fn main() -> Result<()> {
    demarrer_les_traces();

    // Before the window, deliberately: a failure here has to reach the user as
    // a message on stderr. A window that opens onto a broken backend is worse
    // than no window at all.
    let temporary = std::env::args()
        .skip(1)
        .any(|arg| arg == "--temporary-workspace");
    let backend = if temporary {
        Backend::open_temporary()
    } else {
        Backend::open()
    }
    .context("starting Oxyn")?;

    // `with_assets` avant tout : sans lui, `AssetSource::load` n'est jamais
    // consulté et les SVG de `oxyn_ui::icons` ne rendent rien — en silence,
    // puisqu'une ressource absente n'est pas une erreur pour GPUI.
    Application::new().with_assets(UiAssets).run(move |cx| {
        // Et l'enregistrement des polices avant la fenêtre, parce que
        // `Typography::ui_family` nomme « Geist » : une famille non
        // enregistrée tombe sur le repli de plateforme, et l'application
        // dessine dans une autre police que celle de la maquette sans que rien
        // ne le signale. L'échec est rapporté, pas avalé : le repli reste
        // lisible, mais il ne doit pas passer pour le résultat voulu.
        if let Err(erreur) = cx.text_system().add_fonts(UiAssets::fonts()) {
            tracing::error!(%erreur, "Geist non enregistrée : repli de plateforme");
        }
        let quitting_backend = backend.clone();
        cx.on_app_quit(move |cx| {
            cx.set_global(ShutdownStarted);
            let backend = quitting_backend.clone();
            async move {
                backend.wait_for_local_writes().await;
            }
        })
        .detach();
        let closing_backend = backend.clone();
        cx.on_window_closed(move |cx| {
            if cx.windows().is_empty() && cx.try_global::<ShutdownStarted>().is_none() {
                let backend = closing_backend.clone();
                cx.spawn(async move |cx| {
                    backend.wait_for_local_writes().await;
                    let _ = cx.update(|cx| cx.quit());
                })
                .detach();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(LARGEUR), px(HAUTEUR)), cx);
        let ouverture = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // Sans titre explicite, la fenêtre en porte un vide : elle
                // devient impossible à désigner dans Mission Control ou dans le
                // sélecteur de fenêtres, là où il n'y a que le titre à lire.
                titlebar: Some(TitlebarOptions {
                    title: Some(
                        if temporary {
                            "Oxyn · Temporary workspace"
                        } else {
                            "Oxyn"
                        }
                        .into(),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_window, cx| cx.new(|cx| Root::new(backend.clone(), cx)),
        );

        match ouverture {
            Ok(_) => {
                cx.activate(true);
                tracing::info!("window ready");
            }
            // `quit` rather than a panic: the process must not leave a
            // half-initialised window manager state behind it.
            Err(erreur) => {
                tracing::error!(error = %erreur, "the window could not be opened");
                cx.quit();
            }
        }
    });

    Ok(())
}

/// Logging, off by default beyond `info`.
///
/// `OXYN_LOG` and not `RUST_LOG`: the latter is read by every Rust program on
/// the machine, and turning on `debug` for a whole shell should not make Oxyn
/// noisy.
fn demarrer_les_traces() {
    let filtre =
        EnvFilter::try_from_env("OXYN_LOG").unwrap_or_else(|_| EnvFilter::new("oxyn=info,warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filtre)
        .with_target(true)
        .init();
}
