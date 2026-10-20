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
/// First it picks English on the page's language menu, which the page
/// reports at once through an `orag-lang:` navigation that the app cancels;
/// the POST starts right behind it. The POST must still answer and the
/// script still run, so a run shows that a cancelled report leaves the page
/// alone, and that the choice is saved before the app closes its window
/// right after the smoke report (`language` in the app data folder reads
/// `en <ms>`). WebKit keeps one scheduled navigation per page and a newer
/// one replaces it, so the smoke report waits a moment for the language
/// report to be decided first. The page-load event can come while the
/// page's modules are still loading, so it waits for `load`.
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
  await new Promise((resolve) => setTimeout(resolve, 1000));
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

/// The pair the app hands the language to the service page in, in its URL
/// fragment (`#lang=en`), and the pages report a choice in.
const LANGUAGE_PAIR: &str = "lang";

/// When the language was chosen (ms since 1970), next to it.
const AT_PAIR: &str = "at";

/// The pages' language report channel: a page navigates to
/// `orag-lang://choice?lang=<tr|en>&at=<ms>` the moment a choice is made,
/// and the app cancels that navigation and keeps the choice. (The webview's
/// address is never read for it: WKWebView has none while a navigation is
/// pending, and wry 0.57 unwraps it.)
pub const LANGUAGE_SCHEME: &str = "orag-lang";

/// The only host of a language report.
const LANGUAGE_HOST: &str = "choice";

/// The latest time a report may name: the last millisecond of the year 9999,
/// so a sane page clock always fits and `+ 1` never overflows.
pub const LATEST_AT: u64 = 253_402_300_799_999;

/// Whether `url` is on the language report channel (valid or not); such a
/// navigation is always cancelled.
pub fn is_language_report(url: &Url) -> bool {
    url.scheme() == LANGUAGE_SCHEME
}

/// The choice a language report names, or None for anything that is not
/// exactly `orag-lang://choice?lang=<tr|en>&at=<1..=LATEST_AT>`: no other
/// host, port, user, path, fragment, parameter or repeated parameter, and
/// `at` in plain digits.
pub fn language_report(url: &Url) -> Option<Choice> {
    let bare = is_language_report(url)
        && url.host_str() == Some(LANGUAGE_HOST)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().is_empty()
        && url.fragment().is_none();
    if !bare {
        return None;
    }
    // The raw query, never percent-decoded: the pages write plain values.
    let (mut language, mut at) = (None, None);
    for pair in url.query()?.split('&') {
        let (name, value) = pair.split_once('=')?;
        let slot = match name {
            LANGUAGE_PAIR => &mut language,
            AT_PAIR => &mut at,
            _ => return None,
        };
        if slot.replace(value).is_some() {
            return None; // repeated
        }
    }
    let language = crate::prefs::parse_language(language?)?;
    Some(Choice {
        language,
        at: Some(report_time(at?)?),
    })
}

/// A report's time: plain digits, 1 to [`LATEST_AT`].
fn report_time(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u64>()
        .ok()
        .filter(|at| (1..=LATEST_AT).contains(at))
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
    }

    #[test]
    fn a_language_report_names_a_timed_choice() {
        assert_eq!(
            language_report(&url("orag-lang://choice?lang=en&at=1700000000000")),
            Some(choice("en", Some(1_700_000_000_000)))
        );
        assert_eq!(
            language_report(&url("orag-lang://choice?at=1&lang=tr")),
            Some(choice("tr", Some(1)))
        );
        assert_eq!(
            language_report(&url(&format!("orag-lang://choice?lang=en&at={LATEST_AT}"))),
            Some(choice("en", Some(LATEST_AT)))
        );
        assert!(is_language_report(&url("orag-lang://choice?lang=en&at=1")));
        assert!(is_language_report(&url("orag-lang://elsewhere")));
        assert!(!is_language_report(&url("orag-smoke://result?status=400")));
        assert!(!is_language_report(&url(
            "http://127.0.0.1:42000/#lang=en&at=1"
        )));
    }

    #[test]
    fn junk_language_reports_are_refused() {
        let too_late = LATEST_AT + 1;
        for bad in [
            // Wrong scheme, host, port, user, path or fragment.
            "orag-smoke://choice?lang=en&at=1".to_string(),
            "http://choice/?lang=en&at=1".to_string(),
            "http://127.0.0.1:42000/?lang=en&at=1".to_string(),
            "orag-lang://other?lang=en&at=1".to_string(),
            "orag-lang://CHOICE?lang=en&at=1".to_string(),
            "orag-lang://choice:80?lang=en&at=1".to_string(),
            "orag-lang://me@choice?lang=en&at=1".to_string(),
            "orag-lang://choice/x?lang=en&at=1".to_string(),
            "orag-lang://choice/?lang=en&at=1".to_string(),
            "orag-lang://choice?lang=en&at=1#x".to_string(),
            "orag-lang:choice?lang=en&at=1".to_string(),
            // Missing or bad language.
            "orag-lang://choice".to_string(),
            "orag-lang://choice?at=1".to_string(),
            "orag-lang://choice?lang=&at=1".to_string(),
            "orag-lang://choice?lang=de&at=1".to_string(),
            "orag-lang://choice?lang=EN&at=1".to_string(),
            "orag-lang://choice?lang=en%00&at=1".to_string(),
            "orag-lang://choice?lang=e%6E&at=1".to_string(),
            // Missing, untimed, zero, negative, huge or non-numeric time.
            "orag-lang://choice?lang=en".to_string(),
            "orag-lang://choice?lang=en&at=".to_string(),
            "orag-lang://choice?lang=en&at=0".to_string(),
            "orag-lang://choice?lang=en&at=-5".to_string(),
            "orag-lang://choice?lang=en&at=+5".to_string(),
            "orag-lang://choice?lang=en&at=1.5".to_string(),
            "orag-lang://choice?lang=en&at=1e3".to_string(),
            "orag-lang://choice?lang=en&at=soon".to_string(),
            "orag-lang://choice?lang=en&at=0x10".to_string(),
            "orag-lang://choice?lang=en&at=%31".to_string(),
            "orag-lang://choice?lang=en&at= 1".to_string(),
            format!("orag-lang://choice?lang=en&at={too_late}"),
            "orag-lang://choice?lang=en&at=18446744073709551616".to_string(),
            "orag-lang://choice?lang=en&at=99999999999999999999999".to_string(),
            // Extra, repeated or malformed parameters.
            "orag-lang://choice?lang=en&at=1&x=1".to_string(),
            "orag-lang://choice?lang=en&at=1&".to_string(),
            "orag-lang://choice?lang=en&lang=tr&at=1".to_string(),
            "orag-lang://choice?lang=en&at=1&at=2".to_string(),
            "orag-lang://choice?lang=en&at".to_string(),
            "orag-lang://choice?lang=en;at=1".to_string(),
        ] {
            assert_eq!(language_report(&url(&bad)), None, "{bad}");
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
