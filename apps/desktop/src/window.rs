//! What the window shows and where it may go, as pure functions.

use serde::Serialize;
use tauri::Url;

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
    fn only_web_links_open_externally() {
        assert!(external_allowed(&url("https://github.com/orkono/orag")));
        assert!(!external_allowed(&url("file:///etc/passwd")));
        assert!(!external_allowed(&url("javascript:alert(1)")));
    }
}
