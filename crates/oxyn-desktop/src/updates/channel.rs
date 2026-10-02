//! Where updates come from: the only endpoint in the code, and the release
//! page built from a fixed prefix. A beta channel would be a second constant
//! here, chosen by the preference — never a URL the webview sends.

/// The stable channel's manifest: the latest published GitHub release.
/// A draft is not served by `/releases/latest`.
pub(crate) const STABLE_MANIFEST: &str =
    "https://github.com/so-keyldzn/oxyn/releases/latest/download/latest.json";

/// The release pages' prefix, completed by a validated version.
const RELEASE_PAGE: &str = "https://github.com/so-keyldzn/oxyn/releases/tag/v";

/// Whether an archive may be downloaded from `url`. The plugin enforces
/// HTTPS on the endpoint only, not on the URL the manifest announces; the
/// signature still guards the bytes, this guards where they are asked from.
/// GitHub redirects the download to its storage, which `reqwest` follows.
pub(crate) fn is_release_asset(url: &tauri::Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("github.com")
}

/// The page of `version`'s release. `None` unless `version` is a plain
/// semantic version: it ends up in a URL opened by the system.
pub(crate) fn release_page(version: &str) -> Option<String> {
    is_semver(version).then(|| format!("{RELEASE_PAGE}{version}"))
}

/// `MAJOR.MINOR.PATCH`, numeric, then an optional `-pre` and `+build` of
/// ASCII letters, digits, dots and hyphens — the grammar of semver.org,
/// without its finer rules, which a URL does not need.
pub(crate) fn is_semver(version: &str) -> bool {
    const LONGEST: usize = 128;
    if version.is_empty() || version.len() > LONGEST {
        return false;
    }
    let (rest, build) = match version.split_once('+') {
        Some((rest, build)) => (rest, Some(build)),
        None => (version, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let numbers: Vec<&str> = core.split('.').collect();
    let core_ok = numbers.len() == 3
        && numbers
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    let label_ok = |label: &str| {
        !label.is_empty()
            && label.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
    };
    core_ok && pre.is_none_or(label_ok) && build.is_none_or(label_ok)
}
