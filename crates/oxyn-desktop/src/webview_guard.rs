//! The main window, built so that its webview cannot become a browser.
//!
//! Declared `"create": false` in `tauri.conf.json` and built here from that
//! declaration, because the two guards ADR-0041 § 8 asks for — refusing a
//! navigation away from the application and refusing a new window — are
//! only offered on the builder. Without the second, WebView2 answers
//! `window.open` or a `target="_blank"` link with a browser popup
//! ([UX-SPEC](../../../docs/UX-SPEC.md), « Ce qu'Oxyn ne fait pas, parce que
//! ce n'est pas un navigateur »).
//!
//! The front never renders an external link; these are the last line, for
//! the link or script that gets through anyway.

use anyhow::{Context as _, Result};
use tauri::webview::NewWindowResponse;
use tauri::{App, Url, WebviewWindowBuilder};

/// Builds the window `tauri.conf.json` declares under `label`, with its title.
///
/// Runs in `setup`, on the main thread, where creating a window does not
/// deadlock (the warning on `WebviewWindowBuilder::new` is about synchronous
/// commands and event handlers).
pub(crate) fn open_window(app: &App, label: &str, title: &str) -> Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == label)
        .with_context(|| format!("tauri.conf.json declares no window {label:?}"))?;
    let origin = app_origin(
        tauri::is_dev(),
        app.config().build.dev_url.as_ref(),
        config.use_https_scheme,
    )
    .context("resolving the application's own origin")?;
    WebviewWindowBuilder::from_config(app, config)?
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
        .with_context(|| format!("opening the {label:?} window"))?;
    Ok(())
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

    use super::{app_origin, same_origin};

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
}
