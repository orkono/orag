//! The Tauri application: one window, one supervised sidecar (D-016).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindow};

use crate::supervisor::{SidecarEvent, SidecarSpec, Supervisor};
use crate::window::{
    Action, Status, action_for, external_allowed, navigation_allowed, status_script,
};

/// Set in a smoke test: the app writes `<ui url> <child pid>` here on Ready.
pub const READY_FILE_ENV: &str = "ORAG_DESKTOP_READY_FILE";

/// The app's own page (its origin differs by OS: `tauri://localhost`,
/// `http://tauri.localhost`) and the service page currently allowed (it
/// moves to a new port on every restart).
#[derive(Default)]
struct Pages {
    local: Mutex<Option<Url>>,
    service: Mutex<Option<Url>>,
}

fn locked<T: Clone>(m: &Mutex<T>) -> T {
    m.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set<T>(m: &Mutex<T>, value: T) {
    *m.lock().unwrap_or_else(|e| e.into_inner()) = value;
}

#[derive(Default)]
struct Sidecar(Mutex<Option<Supervisor>>);

pub fn run() {
    let app = tauri::Builder::default()
        // First, so a second launch exits before it starts anything.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(Pages::default()))
        .manage(Sidecar::default())
        .setup(|app| {
            let window = build_window(app.handle())?;
            start_sidecar(app.handle(), window);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("the ORAG desktop app failed to start");
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            stop_sidecar(handle);
        }
    });
}

fn build_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let pages = app.state::<Arc<Pages>>().inner().clone();
    let nav_pages = pages.clone();
    let nav_app = app.clone();
    let new_app = app.clone();
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("ORAG")
        .inner_size(1100.0, 780.0)
        .min_inner_size(480.0, 400.0)
        .on_navigation(move |url| {
            // Before the first page load the app's own URL is not known yet.
            let allowed = locked(&nav_pages.local).as_ref().is_none_or(|local| {
                navigation_allowed(url, local, locked(&nav_pages.service).as_ref())
            });
            if !allowed {
                open_external(&nav_app, url);
            }
            allowed
        })
        .on_new_window(move |url, _features| {
            open_external(&new_app, &url);
            NewWindowResponse::Deny
        })
        .build()?;
    set(&pages.local, window.url().ok());
    Ok(window)
}

fn open_external(app: &AppHandle, url: &Url) {
    if external_allowed(url) {
        use tauri_plugin_opener::OpenerExt;
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

fn start_sidecar(app: &AppHandle, window: WebviewWindow) {
    let spec = match sidecar_spec(app) {
        Ok(spec) => spec,
        Err(message) => {
            show(
                app,
                &window,
                &Status::Failed {
                    message,
                    details: Vec::new(),
                },
            );
            return;
        }
    };
    // Stored before its events are read, so a Ready finds its child pid.
    let events = {
        let sidecar = app.state::<Sidecar>();
        let mut slot = sidecar.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.insert(Supervisor::start(spec)).take_events()
    };
    let app = app.clone();
    std::thread::spawn(move || {
        for event in events {
            handle_event(&app, &window, &event);
        }
    });
}

fn sidecar_spec(app: &AppHandle) -> Result<SidecarSpec, String> {
    let exe = std::env::current_exe().map_err(|e| format!("locating the app: {e}"))?;
    let dev = std::env::var(crate::sidecar::DEV_SIDECAR_ENV).ok();
    let program = crate::sidecar::resolve(&exe, dev.as_deref(), cfg!(debug_assertions))?;
    let data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("finding the app data folder: {e}"))?;
    let home: PathBuf = crate::home::ensure_home(&data)
        .map_err(|e| format!("preparing {}: {e}", data.display()))?;
    Ok(SidecarSpec::serve(program, home))
}

fn handle_event(app: &AppHandle, window: &WebviewWindow, event: &SidecarEvent) {
    let pages = app.state::<Arc<Pages>>();
    match action_for(event) {
        Action::Open(url) => {
            set(&pages.service, Some(url.clone()));
            write_ready_file(app, &url);
            let _ = window.navigate(url);
        }
        Action::Show(status) => {
            set(&pages.service, None);
            show(app, window, &status);
        }
    }
}

/// Shows a status on the app's own page: handed over directly when that page
/// is loaded, otherwise in the URL fragment of a fresh load (`ui/index.html`
/// reads it).
fn show(app: &AppHandle, window: &WebviewWindow, status: &Status) {
    let Some(local) = locked(&app.state::<Arc<Pages>>().local) else {
        return;
    };
    let on_local = window
        .url()
        .is_ok_and(|current| navigation_allowed(&current, &local, None));
    if on_local {
        let _ = window.eval(status_script(status));
        return;
    }
    let mut url = local;
    url.set_fragment(Some(&urlencode(
        &serde_json::to_string(status).unwrap_or_default(),
    )));
    let _ = window.navigate(url);
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn write_ready_file(app: &AppHandle, url: &Url) {
    let Ok(path) = std::env::var(READY_FILE_ENV) else {
        return;
    };
    let pid = app
        .state::<Sidecar>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(Supervisor::child_pid)
        .unwrap_or(0);
    let _ = std::fs::write(path, format!("{url} {pid}\n"));
}

fn stop_sidecar(app: &AppHandle) {
    let supervisor = app
        .state::<Sidecar>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(supervisor) = supervisor {
        supervisor.shutdown();
    }
}
