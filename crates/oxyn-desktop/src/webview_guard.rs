//! The windows, built so that their webview cannot become a browser.
//!
//! `tauri.conf.json` declares one window, `workspace`, with `"create": false`:
//! a template. Every window, the first included, is built here from it under
//! the label Rust chose (`backend/windows.rs`), so that dimensions, minimum
//! size and title bar stay written in one place
//! ([ADR-0043](../../../docs/adr/0043-multi-fenetre.md)). And built here
//! because the two guards ADR-0041 § 8 asks for — refusing a navigation away
//! from the application and refusing a new window — are only offered on the
//! builder. Without the second, WebView2 answers `window.open` or a
//! `target="_blank"` link with a browser popup
//! ([UX-SPEC](../../../docs/UX-SPEC.md), « Ce qu'Oxyn ne fait pas, parce que
//! ce n'est pas un navigateur »).
//!
//! The front never renders an external link; these are the last line, for
//! the link or script that gets through anyway.

use anyhow::{Context as _, Result};
use oxyn_core::WindowGeometry;
use tauri::utils::config::WindowConfig;
use tauri::webview::NewWindowResponse;
use tauri::{Manager, Runtime, Url, WebviewWindow, WebviewWindowBuilder};

/// The window `tauri.conf.json` declares as the template of every window.
pub(crate) const TEMPLATE: &str = "workspace";

/// A screen's usable rectangle, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Screen {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// The screens connected now, in logical pixels, each at its own scale.
pub(crate) fn screens(app: &tauri::AppHandle) -> Vec<Screen> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .filter(|monitor| monitor.scale_factor().is_finite() && monitor.scale_factor() > 0.0)
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            Screen {
                x: f64::from(area.position.x) / scale,
                y: f64::from(area.position.y) / scale,
                width: f64::from(area.size.width) / scale,
                height: f64::from(area.size.height) / scale,
            }
        })
        .collect()
}

/// Applies what the workspace file kept of a window to the template.
///
/// The file is hostile input (ADR-0043): a size under the template's minimum
/// is brought up to it, and a rectangle that meets no screen connected now
/// is dropped — the system places the window, at the template's size, rather
/// than opening it where nobody can reach it.
fn restore(config: &mut WindowConfig, geometry: &WindowGeometry, screens: &[Screen]) {
    let width = geometry.width.max(config.min_width.unwrap_or(0.0));
    let height = geometry.height.max(config.min_height.unwrap_or(0.0));
    config.maximized = geometry.maximized;
    let Some((x, y)) = geometry.x.zip(geometry.y) else {
        config.width = width;
        config.height = height;
        return;
    };
    let meets = screens.iter().any(|screen| {
        x < screen.x + screen.width
            && screen.x < x + width
            && y < screen.y + screen.height
            && screen.y < y + height
    });
    if meets {
        config.width = width;
        config.height = height;
        config.x = Some(x);
        config.y = Some(y);
        config.center = false;
    }
}

/// Builds a window from the template, under `label`, with its title, where
/// `restored` left it when it comes back from the workspace file.
///
/// Never from a synchronous command or event handler: creating a window
/// there deadlocks on Windows (the warning on `WebviewWindowBuilder::new`).
/// From `setup`, on the main thread, or from an `async` command.
pub(crate) fn open_window<R: Runtime, M: Manager<R>>(
    manager: &M,
    label: &str,
    title: &str,
    restored: Option<(&WindowGeometry, &[Screen])>,
) -> Result<WebviewWindow<R>> {
    let mut config = manager
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == TEMPLATE)
        .with_context(|| format!("tauri.conf.json declares no window {TEMPLATE:?}"))?
        .clone();
    config.label = label.to_owned();
    if let Some((geometry, screens)) = restored {
        restore(&mut config, geometry, screens);
    }
    let origin = app_origin(
        tauri::is_dev(),
        manager.config().build.dev_url.as_ref(),
        config.use_https_scheme,
    )
    .context("resolving the application's own origin")?;
    WebviewWindowBuilder::from_config(manager, &config)?
        .title(title)
        .on_navigation(move |url| {
            let allowed = same_origin(url, &origin);
            if !allowed {
                // Scheme and host only: a URL's path or query can carry
                // whatever the page that built it put there (I-03).
                tracing::warn!(
                    scheme = url.scheme(),
                    host = url.host_str().unwrap_or(""),
                    "navigation away from the application refused"
                );
            }
            allowed
        })
        .on_new_window(|url, _features| {
            tracing::warn!(
                scheme = url.scheme(),
                host = url.host_str().unwrap_or(""),
                "new webview window refused"
            );
            NewWindowResponse::Deny
        })
        .build()
        .with_context(|| format!("opening the {label:?} window"))
}

/// The origin the application's pages are served from.
///
/// In development, Vite's `devUrl`; in a build, Tauri's asset protocol, which
/// WebView2 cannot register as a scheme and serves as `http(s)://tauri.localhost`
/// (`tauri` `manager/mod.rs`, `tauri_protocol_url`). The Windows form is only
/// accepted on Windows: elsewhere `tauri.localhost` is an ordinary loopback
/// host, where any local server could answer.
fn app_origin(dev: bool, dev_url: Option<&Url>, https: bool) -> Result<Url> {
    if dev {
        return dev_url
            .cloned()
            .context("a development build without devUrl in tauri.conf.json");
    }
    let origin = match (cfg!(windows), https) {
        (true, true) => "https://tauri.localhost",
        (true, false) => "http://tauri.localhost",
        (false, _) => "tauri://localhost",
    };
    Ok(Url::parse(origin)?)
}

/// Whether `url` is served from `origin`.
///
/// Compared field by field rather than by `Url::origin`: a custom scheme such
/// as `tauri:` has an opaque origin, which equals no other — not even itself
/// parsed twice.
fn same_origin(url: &Url, origin: &Url) -> bool {
    url.scheme() == origin.scheme()
        && url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tauri::Url;

    use oxyn_core::WindowGeometry;
    use tauri::utils::config::WindowConfig;

    use super::{Screen, app_origin, restore, same_origin};

    fn template() -> WindowConfig {
        WindowConfig {
            width: 1280.0,
            height: 820.0,
            min_width: Some(960.0),
            min_height: Some(600.0),
            ..WindowConfig::default()
        }
    }

    const LAPTOP: Screen = Screen {
        x: 0.0,
        y: 25.0,
        width: 1512.0,
        height: 920.0,
    };

    fn kept(x: Option<f64>, width: f64) -> WindowGeometry {
        WindowGeometry {
            x,
            y: x,
            width,
            height: 700.0,
            maximized: false,
        }
    }

    #[test]
    fn a_window_comes_back_where_it_was_on_a_connected_screen() {
        let mut config = template();
        restore(&mut config, &kept(Some(100.0), 1100.0), &[LAPTOP]);
        assert_eq!((config.x, config.y), (Some(100.0), Some(100.0)));
        assert_eq!((config.width, config.height), (1100.0, 700.0));
        assert!(!config.center);
    }

    #[test]
    fn a_size_under_the_minimum_is_brought_up_to_it() {
        let mut config = template();
        restore(&mut config, &kept(None, 0.0), &[LAPTOP]);
        assert_eq!((config.width, config.height), (960.0, 700.0));
        assert_eq!(config.x, None);
    }

    /// The screen it stood on was unplugged: the system places it, at the
    /// template's size, rather than off every screen.
    #[test]
    fn a_window_off_every_screen_is_placed_by_the_system() {
        let mut config = template();
        restore(&mut config, &kept(Some(4000.0), 1100.0), &[LAPTOP]);
        assert_eq!(config.x, None);
        assert_eq!((config.width, config.height), (1280.0, 820.0));
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("the test URLs are valid")
    }

    #[rstest]
    #[case::the_start_page("tauri://localhost/index.html")]
    #[case::a_route("tauri://localhost/workspace?console=2#top")]
    fn a_build_stays_on_its_own_pages(#[case] target: &str) {
        let origin = app_origin(false, None, false).expect("a constant origin");
        if cfg!(windows) {
            assert!(!same_origin(&url(target), &origin));
        } else {
            assert!(same_origin(&url(target), &origin));
        }
    }

    #[rstest]
    #[case::https("https://example.com/")]
    #[case::http("http://example.com/")]
    #[case::a_loopback_server("http://localhost:3000/")]
    #[case::a_local_file("file:///etc/passwd")]
    #[case::script("javascript:alert(1)")]
    #[case::data("data:text/html,<p>hi</p>")]
    #[case::blank("about:blank")]
    #[case::another_tauri_host("tauri://evil/")]
    fn a_build_refuses_everything_else(#[case] target: &str) {
        let origin = app_origin(false, None, false).expect("a constant origin");
        assert!(!same_origin(&url(target), &origin));
    }

    #[test]
    fn tauri_localhost_is_the_application_only_on_windows() {
        let origin = app_origin(false, None, false).expect("a constant origin");
        assert_eq!(
            same_origin(&url("http://tauri.localhost/index.html"), &origin),
            cfg!(windows)
        );
    }

    #[test]
    fn development_follows_the_dev_server_and_nothing_else() {
        let dev = url("http://localhost:3000");
        let origin = app_origin(true, Some(&dev), false).expect("devUrl is set");
        assert!(same_origin(
            &url("http://localhost:3000/workspace"),
            &origin
        ));
        assert!(!same_origin(&url("http://localhost:3001/"), &origin));
        assert!(!same_origin(&url("https://localhost:3000/"), &origin));
        assert!(!same_origin(&url("tauri://localhost/"), &origin));
    }

    #[test]
    fn development_without_a_dev_server_is_an_error() {
        assert!(app_origin(true, None, false).is_err());
    }

    /// Every window is built from one template, and the capability covers
    /// them by a pattern with exactly the three permissions it had: none
    /// lets a webview create a window or a webview (ADR-0043).
    #[test]
    fn every_window_comes_from_the_template_with_the_same_three_permissions() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid JSON");
        let windows = config["app"]["windows"].as_array().expect("windows");
        assert_eq!(
            windows.len(),
            1,
            "one template, no window built by Tauri itself"
        );
        assert_eq!(windows[0]["label"], super::TEMPLATE);
        assert_eq!(windows[0]["create"], false);

        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).expect("valid JSON");
        assert_eq!(capability["windows"], serde_json::json!(["workspace-*"]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "core:window:allow-start-dragging",
                "core:window:allow-internal-toggle-maximize",
                "dialog:allow-open"
            ])
        );
        assert!(capability.get("webviews").is_none());
    }
}
