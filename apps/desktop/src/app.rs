//! The Tauri application: one window, one supervised sidecar (D-016).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindow};

use crate::supervisor::{SidecarEvent, SidecarSpec, Supervisor};
use crate::window::{
    Action, SMOKE_SCRIPT, Status, action_for, external_allowed, local_origin, navigation_allowed,
    smoke_report, status_script,
};

/// Set in a smoke test: the app writes `<ui url> <child pid>` here on Ready.
pub const READY_FILE_ENV: &str = "ORAG_DESKTOP_READY_FILE";
/// Set in a smoke test: once the service page has loaded, it sends one
/// same-origin POST from inside the webview; the app writes the result here
/// and closes its window, exercising the normal close path.
pub const SMOKE_FILE_ENV: &str = "ORAG_DESKTOP_SMOKE_FILE";

/// What the window may show: the app's own page (origin fixed per OS) and
/// the service page currently allowed (it moves on every restart), plus the
/// last status, handed to the app page whenever it finishes loading.
struct Pages {
    local: Url,
    service: Mutex<Option<Url>>,
    status: Mutex<Status>,
    smoke_file: Option<PathBuf>,
    smoke_sent: AtomicBool,
    /// The app page has finished loading and is still shown. Tracked here
    /// rather than read from the webview, whose URL is not available while
    /// a page is still loading.
    on_local: AtomicBool,
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
    let pages = Arc::new(Pages {
        local: local_origin(),
        service: Mutex::new(None),
        status: Mutex::new(Status::Loading),
        smoke_file: std::env::var_os(SMOKE_FILE_ENV).map(PathBuf::from),
        smoke_sent: AtomicBool::new(false),
        on_local: AtomicBool::new(false),
    });
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
        .manage(pages)
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
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("ORAG")
        .inner_size(1100.0, 780.0)
        .min_inner_size(480.0, 400.0)
        .on_navigation(move |url| {
            if let Some(report) = smoke_report(url) {
                record_smoke(&nav_app, &nav_pages, &report);
                return false;
            }
            let allowed =
                navigation_allowed(url, &nav_pages.local, locked(&nav_pages.service).as_ref());
            if !allowed {
                open_external(&nav_app, url);
            }
            allowed
        })
        .on_new_window(move |url, _features| {
            open_external(&new_app, &url);
            NewWindowResponse::Deny
        })
        .on_page_load(move |window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                page_loaded(&window, &pages, payload.url());
            }
        })
        .build()
}

/// The app page gets the current status once its script is there; the
/// service page runs the smoke check when one was asked for.
fn page_loaded(window: &WebviewWindow, pages: &Pages, url: &Url) {
    let local = navigation_allowed(url, &pages.local, None);
    pages.on_local.store(local, Ordering::SeqCst);
    if local {
        let _ = window.eval(status_script(&locked(&pages.status)));
        return;
    }
    let on_service = locked(&pages.service)
        .as_ref()
        .is_some_and(|service| navigation_allowed(url, &pages.local, Some(service)));
    if on_service && pages.smoke_file.is_some() && !pages.smoke_sent.swap(true, Ordering::SeqCst) {
        let _ = window.eval(SMOKE_SCRIPT);
    }
}

fn record_smoke(app: &AppHandle, pages: &Pages, report: &str) {
    if let Some(path) = &pages.smoke_file {
        let _ = std::fs::write(path, format!("{report}\n"));
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.close();
    }
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
                Status::Failed {
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
    match action_for(event) {
        Action::Open(url) => {
            let pages = app.state::<Arc<Pages>>();
            pages.on_local.store(false, Ordering::SeqCst);
            set(&pages.service, Some(url.clone()));
            write_ready_file(app, &url);
            let _ = window.navigate(url);
        }
        Action::Show(status) => show(app, window, status),
    }
}

/// Records the status and shows it on the app page: handed over at once when
/// that page is up, otherwise by its page-load hook after a fresh load.
fn show(app: &AppHandle, window: &WebviewWindow, status: Status) {
    let pages = app.state::<Arc<Pages>>();
    set(&pages.service, None);
    set(&pages.status, status.clone());
    if pages.on_local.load(Ordering::SeqCst) {
        let _ = window.eval(status_script(&status));
    } else if let Ok(index) = pages.local.join("index.html") {
        let _ = window.navigate(index);
    }
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
