//! What the window shows and where it may go, as pure functions.

use serde::Serialize;
use tauri::Url;

use crate::prefs::Choice;
use crate::supervisor::SidecarEvent;

/// What the local status page (`ui/index.html`) shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Status {
    Loading,
    Restarting {
        reason: String,
    },
    Failed {
        message: String,
        details: Vec<String>,
    },
}

/// The origin Tauri serves the app's own files from (`ui/`): fixed per OS,
/// so it is known before the first page loads.
pub fn local_origin() -> Url {
    let origin = if cfg!(windows) {
        "http://tauri.localhost/"
    } else {
        "tauri://localhost/"
    };
    Url::parse(origin).expect("a valid constant URL")
}

/// The smoke test's report channel: the service page navigates here with the
/// result of its own API call, and the app records it (`ORAG_DESKTOP_SMOKE_FILE`).
pub const SMOKE_SCHEME: &str = "orag-smoke";

/// The script the smoke test runs inside the service page: a same-origin POST
/// as the page itself sends it, reported through a `orag-smoke:` navigation.
/// It also picks English on the page's language menu first: the app closes
/// its window right after the report, well before the next language poll,
/// so a run shows that the close path saves the last choice (`language` in
/// the app data folder reads `en <ms>` afterwards). The page-load event can
/// come while the page's modules are still loading, so it waits for `load`.
pub const SMOKE_SCRIPT: &str = r#"(async () => {
  if (document.readyState !== "complete") {
    await new Promise((resolve) => window.addEventListener("load", resolve, { once: true }));
  }
  const menu = document.getElementById("language-select");
  if (menu) {
    menu.value = "en";
    menu.dispatchEvent(new Event("change"));
  }
  let report;
  try {
    const r = await fetch("/v1/models/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path: "relative/pack" }),
    });
    report = "status=" + r.status + "&body=" + encodeURIComponent((await r.text()).slice(0, 300));
  } catch (e) {
    report = "error=" + encodeURIComponent(String(e));
  }
  window.location.href = "orag-smoke://result?" + report;
})();"#;

/// The smoke report as text (`<status> <body>` or `error <message>`).
pub fn smoke_report(url: &Url) -> Option<String> {
    if url.scheme() != SMOKE_SCHEME {
        return None;
    }
    let pairs: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    Some(match (pairs.get("status"), pairs.get("error")) {
        (Some(status), _) => format!("{status} {}", pairs.get("body").map_or("", String::as_str)),
        (None, Some(error)) => format!("error {error}"),
        (None, None) => "error no result".to_string(),
    })
}

/// What the window should do for a supervisor event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Load the service page.
    Open(Url),
    /// Show the local status page with this status.
    Show(Status),
}

pub fn action_for(event: &SidecarEvent) -> Action {
    match event {
        SidecarEvent::Ready { ui, .. } => match Url::parse(ui) {
            Ok(url) if is_loopback_http(&url) => Action::Open(url),
            _ => Action::Show(Status::Failed {
                message: format!("the service reported an unexpected page address: {ui}"),
                details: Vec::new(),
            }),
        },
        SidecarEvent::Restarting { reason } => Action::Show(Status::Restarting {
            reason: reason.clone(),
        }),
        SidecarEvent::Failed {
            message,
            stderr_tail,
        } => Action::Show(Status::Failed {
            message: message.clone(),
            details: stderr_tail.clone(),
        }),
    }
}

/// The script that hands a status to the local page. The status is JSON, so
/// service output (stderr lines) reaches the page as data, never as code;
/// the page renders it with `textContent`.
pub fn status_script(status: &Status) -> String {
    let json = serde_json::to_string(status)
        .unwrap_or_else(|_| "{\"state\":\"loading\"}".into())
        // Valid JSON and JS either way; no `</script>` or line separator can
        // survive into the script even if it is ever embedded in HTML.
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!("window.oragStatus && window.oragStatus({json});")
}

/// The fragment pair both pages keep their language in (`#lang=en`): the app
/// hands the language over in it and reads the page's choice back from it.
const LANGUAGE_PAIR: &str = "lang";

/// When the language was chosen (ms since 1970), next to it in the fragment.
const AT_PAIR: &str = "at";

/// The choice a page's address names in its fragment (`lang=en&at=<ms>`),
/// or None.
pub fn language_of(url: &Url) -> Option<Choice> {
    let pairs: Vec<(&str, &str)> = url
        .fragment()?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .collect();
    let value = |name: &str| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
    let language = crate::prefs::parse_language(value(LANGUAGE_PAIR)?)?;
    let at = value(AT_PAIR).and_then(|at| at.parse::<u64>().ok());
    Some(Choice { language, at })
}

/// `url` with its fragment set to `lang=<language>[&at=<ms>]`.
pub fn with_language(url: &Url, choice: Choice) -> Url {
    let mut url = url.clone();
    let fragment = match choice.at {
        Some(at) => format!("{LANGUAGE_PAIR}={}&{AT_PAIR}={at}", choice.language),
        None => format!("{LANGUAGE_PAIR}={}", choice.language),
    };
    url.set_fragment(Some(&fragment));
    url
}

/// The script that hands the choice to the app's own page (its first load
/// has no fragment to read it from). The language is one of the known ones
/// and the time a number, never page output.
pub fn language_script(choice: Choice) -> String {
    let language = crate::prefs::parse_language(choice.language).unwrap_or("tr");
    let at = choice
        .at
        .map_or_else(|| "null".to_string(), |at| at.to_string());
    format!("window.oragLanguage && window.oragLanguage(\"{language}\", {at});")
}

/// Navigation stays on the app's own page or the service page; anything else
/// opens in the system browser.
pub fn navigation_allowed(url: &Url, local: &Url, service: Option<&Url>) -> bool {
    same_origin(url, local) || service.is_some_and(|s| same_origin(url, s))
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

fn is_loopback_http(url: &Url) -> bool {
    url.scheme() == "http" && url.host_str() == Some("127.0.0.1") && url.port().is_some()
}

/// Links the system browser may open: web pages only, never files or apps.
pub fn external_allowed(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "mailto")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn ready_opens_the_service_page() {
        let event = SidecarEvent::Ready {
            api: "http://127.0.0.1:41000".into(),
            ui: "http://127.0.0.1:42000".into(),
        };
        assert_eq!(
            action_for(&event),
            Action::Open(url("http://127.0.0.1:42000/"))
        );
    }

    #[test]
    fn a_non_loopback_page_address_is_refused() {
        let event = SidecarEvent::Ready {
            api: "http://127.0.0.1:41000".into(),
            ui: "http://example.com:42000".into(),
        };
        assert!(matches!(
            action_for(&event),
            Action::Show(Status::Failed { .. })
        ));
    }

    #[test]
    fn failure_details_reach_the_page_as_data() {
        let status = Status::Failed {
            message: "boom".into(),
            details: vec![
                "</script><script>alert(1)</script>".into(),
                "\"); evil(\"".into(),
            ],
        };
        let script = status_script(&status);
        assert!(!script.contains("</script>"), "{script}");
        let json = script
            .strip_prefix("window.oragStatus && window.oragStatus(")
            .and_then(|s| s.strip_suffix(");"))
            .unwrap();
        let back: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(back["state"], "failed");
        assert_eq!(back["details"][0], "</script><script>alert(1)</script>");
    }

    #[test]
    fn navigation_is_limited_to_the_app_and_the_service() {
        let local = url("tauri://localhost/index.html");
        let service = url("http://127.0.0.1:42000/");
        assert!(navigation_allowed(
            &url("tauri://localhost/"),
            &local,
            Some(&service)
        ));
        assert!(navigation_allowed(
            &url("http://127.0.0.1:42000/v1/x"),
            &local,
            Some(&service)
        ));
        assert!(!navigation_allowed(
            &url("http://127.0.0.1:42001/"),
            &local,
            Some(&service)
        ));
        assert!(!navigation_allowed(
            &url("http://localhost:42000/"),
            &local,
            Some(&service)
        ));
        assert!(!navigation_allowed(
            &url("https://example.com/"),
            &local,
            Some(&service)
        ));
        assert!(!navigation_allowed(
            &url("http://127.0.0.1:42000/"),
            &local,
            None
        ));
    }

    #[test]
    fn local_origin_is_known_up_front() {
        let local = local_origin();
        assert!(navigation_allowed(
            &local.join("index.html").unwrap(),
            &local,
            None
        ));
        assert!(!navigation_allowed(
            &url("http://127.0.0.1:42000/"),
            &local,
            None
        ));
    }

    #[test]
    fn smoke_reports_are_read_from_the_navigation() {
        assert_eq!(
            smoke_report(&url(
                "orag-smoke://result?status=400&body=%7B%22error%22%7D"
            ))
            .unwrap(),
            "400 {\"error\"}"
        );
        assert_eq!(
            smoke_report(&url("orag-smoke://result?error=TypeError")).unwrap(),
            "error TypeError"
        );
        assert!(smoke_report(&url("http://127.0.0.1:42000/")).is_none());
    }

    fn choice(language: &'static str, at: Option<u64>) -> Choice {
        Choice { language, at }
    }

    #[test]
    fn the_language_travels_in_the_fragment() {
        assert_eq!(
            with_language(&url("http://127.0.0.1:42000/"), choice("en", Some(1700))).as_str(),
            "http://127.0.0.1:42000/#lang=en&at=1700"
        );
        assert_eq!(
            with_language(
                &url("http://127.0.0.1:42000/#lang=en&at=1"),
                choice("tr", None)
            )
            .as_str(),
            "http://127.0.0.1:42000/#lang=tr"
        );
        assert_eq!(
            language_of(&url("http://127.0.0.1:42000/#lang=en&at=1700")),
            Some(choice("en", Some(1700)))
        );
        assert_eq!(
            language_of(&url("tauri://localhost/index.html#x=1&lang=tr")),
            Some(choice("tr", None))
        );
        assert_eq!(
            language_of(&url("http://127.0.0.1:42000/#lang=en&at=soon")),
            Some(choice("en", None))
        );
        for bad in [
            "http://127.0.0.1:42000/",
            "http://127.0.0.1:42000/#lang=de",
            "http://127.0.0.1:42000/#lang=EN",
            "http://127.0.0.1:42000/?lang=en",
        ] {
            assert_eq!(language_of(&url(bad)), None, "{bad}");
        }
    }

    #[test]
    fn the_status_page_gets_the_language_as_data() {
        assert_eq!(
            language_script(choice("en", Some(1700))),
            "window.oragLanguage && window.oragLanguage(\"en\", 1700);"
        );
        assert_eq!(
            language_script(choice("tr", None)),
            "window.oragLanguage && window.oragLanguage(\"tr\", null);"
        );
    }

    #[test]
    fn only_web_links_open_externally() {
        assert!(external_allowed(&url("https://github.com/orkono/orag")));
        assert!(!external_allowed(&url("file:///etc/passwd")));
        assert!(!external_allowed(&url("javascript:alert(1)")));
    }
}
