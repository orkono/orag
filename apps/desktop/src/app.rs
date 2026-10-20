//! The Tauri application: one window, one supervised sidecar (D-016).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindow};

use crate::supervisor::{SidecarEvent, SidecarSpec, Supervisor};
use crate::window::{
    Action, SMOKE_SCRIPT, Status, action_for, external_allowed, language_of, language_script,
    local_origin, navigation_allowed, smoke_report, status_script, with_language,
};

/// The pages' language until one was chosen (D-016 pages open in Turkish).
const DEFAULT_LANGUAGE: &str = "tr";

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
    /// The app data folder, where the page port and the language are kept
    /// (`prefs`); None until it is known.
    data: Mutex<Option<PathBuf>>,
    /// The language last chosen on either page. Both pages keep it in their
    /// address (`#lang=`), which is read back whenever the window leaves a
    /// page or closes, and handed to the next page shown.
    language: Mutex<&'static str>,
}

impl Pages {
    /// Takes over the language the shown page names in its address, and
    /// remembers it for the next launch.
    fn capture_language(&self, window: &WebviewWindow) {
        let Some(language) = window.url().ok().as_ref().and_then(language_of) else {
            return;
        };
        if language == locked(&self.language) {
            return;
        }
        set(&self.language, language);
        if let Some(data) = locked(&self.data) {
            let _ = crate::prefs::write_language(&data, language);
        }
    }

    fn remember_ui_port(&self, url: &Url) {
        if let (Some(port), Some(data)) = (url.port(), locked(&self.data)) {
            let _ = crate::prefs::write_ui_port(&data, port);
        }
    }
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
        data: Mutex::new(None),
        language: Mutex::new(DEFAULT_LANGUAGE),
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
            watch_language(app.state::<Arc<Pages>>().inner().clone(), window.clone());
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

/// How often the shown page's language is read back while the app runs.
const LANGUAGE_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// Reads the language from the shown page's address now and then, so a
/// choice survives quitting the app. Off the main thread: there the address
/// is read in place, possibly while the window list is in use.
fn watch_language(pages: Arc<Pages>, window: WebviewWindow) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(LANGUAGE_POLL);
            if window.url().is_err() {
                return; // the window is gone
            }
            pages.capture_language(&window);
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
        let _ = window.eval(language_script(locked(&pages.language)));
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
    let pages = app.state::<Arc<Pages>>();
    if let Some(language) = crate::prefs::read_language(&data) {
        set(&pages.language, language);
    }
    let port = crate::prefs::read_ui_port(&data);
    set(&pages.data, Some(data));
    Ok(SidecarSpec::serve(program, home).with_ui_port_hint(port))
}

fn handle_event(app: &AppHandle, window: &WebviewWindow, event: &SidecarEvent) {
    match action_for(event) {
        Action::Open(url) => {
            let pages = app.state::<Arc<Pages>>();
            // A choice made on the status page goes along to the service page.
            pages.capture_language(window);
            pages.on_local.store(false, Ordering::SeqCst);
            set(&pages.service, Some(url.clone()));
            pages.remember_ui_port(&url);
            write_ready_file(app, &url);
            let _ = window.navigate(with_language(&url, locked(&pages.language)));
        }
        Action::Show(status) => show(app, window, status),
    }
}

/// Records the status and shows it on the app page: handed over at once when
/// that page is up, otherwise by its page-load hook after a fresh load.
fn show(app: &AppHandle, window: &WebviewWindow, status: Status) {
    let pages = app.state::<Arc<Pages>>();
    // Leaving the service page: its language goes along to the status page.
    pages.capture_language(window);
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
