//! The Tauri application: one window, one supervised sidecar (D-016).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewWindowBuilder};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindow};

use crate::bench::channel::{self, Shown};
use crate::bench::control::{Control, Step};
use crate::prefs::Choice;
use crate::supervisor::{Backends, SidecarEvent, SidecarSpec, Supervisor};
use crate::window::{
    Action, DESKTOP_MARKER_SCRIPT, SMOKE_SCRIPT, Status, action_for, external_allowed,
    is_language_report, language_report, language_script, local_origin, navigation_allowed,
    smoke_report, status_script, with_language,
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
    /// The app data folder, where the page port and the language are kept
    /// (`prefs`); None until it is known.
    data: Mutex<Option<PathBuf>>,
    /// The service page has finished loading and is still shown.
    on_service: AtomicBool,
    /// The serve spec of this run, for the GPU bench's executables and home.
    spec: Mutex<Option<SidecarSpec>>,
    /// The bench phase and the watched supervisor's generation; events and
    /// bench commands are handled under this lock (`bench::control`).
    bench: Mutex<Control>,
    /// Set by "Durdur" and on quit; the running bench job is killed.
    bench_cancel: Arc<AtomicBool>,
    bench_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The paused supervisor's policy, until "Devam" resumes it.
    paused: Mutex<Option<Box<dyn Backends>>>,
    /// The language last chosen on either page, with when it was chosen.
    /// Each page reports a choice the moment it is made (`orag-lang:`, see
    /// `window::language_report`); the app saves it at once and hands it to
    /// the next page shown. The webview's address is never read for it.
    language: Mutex<Choice>,
}

impl Pages {
    /// Takes over a choice a page reported when it is newer
    /// (`Choice::prefer`), and remembers it for the next launch at once.
    fn take_language(&self, reported: Choice) {
        let current = locked(&self.language);
        let chosen = Choice::prefer(current, reported);
        if chosen == current {
            return;
        }
        set(&self.language, chosen);
        if let Some(data) = locked(&self.data) {
            let _ = crate::prefs::write_language(&data, chosen);
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
        on_service: AtomicBool::new(false),
        spec: Mutex::new(None),
        bench: Mutex::new(Control::default()),
        bench_cancel: Arc::new(AtomicBool::new(false)),
        bench_thread: Mutex::new(None),
        paused: Mutex::new(None),
        data: Mutex::new(None),
        language: Mutex::new(Choice::default()),
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
    // The language is saved when it is chosen, so closing or quitting has
    // nothing to read from the window (whose address WKWebView may not have).
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            stop_bench(handle);
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
        .initialization_script(DESKTOP_MARKER_SCRIPT)
        .on_navigation(move |url| {
            // A page reporting a language choice: kept, never followed.
            if is_language_report(url) {
                if let Some(choice) = language_report(url) {
                    nav_pages.take_language(choice);
                }
                return false;
            }
            if channel::is_bench(url) {
                bench_command(&nav_app, &nav_pages, url);
                return false;
            }
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
    let on_service = !local
        && locked(&pages.service)
            .as_ref()
            .is_some_and(|service| navigation_allowed(url, &pages.local, Some(service)));
    pages.on_service.store(on_service, Ordering::SeqCst);
    if local {
        let _ = window.eval(language_script(locked(&pages.language)));
        let _ = window.eval(status_script(&locked(&pages.status)));
        return;
    }
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
    let generation = {
        let pages = app.state::<Arc<Pages>>();
        let mut control = pages.bench.lock().unwrap_or_else(|e| e.into_inner());
        control.watch()
    };
    start_sidecar_as(app, window, generation);
}

fn start_sidecar_as(app: &AppHandle, window: WebviewWindow, generation: u64) {
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
    set(&app.state::<Arc<Pages>>().spec, Some(spec.clone()));
    // Windows and Linux bundles carry orag-vulkan too: the GPU policy picks
    // one per start and falls back to the CPU (v0.6 plan, G-3).
    let supervisor = match crate::sidecar::vulkan_beside(&spec.program) {
        Some(vulkan) => {
            let home = home_of(&spec);
            let mut gpu = spec.clone();
            gpu.program = vulkan;
            Supervisor::start_with(crate::gpu::GpuBackends::new(spec, gpu, home))
        }
        None => Supervisor::start(spec),
    };
    watch(app, window, supervisor, generation);
}

/// Keeps `supervisor` as the app's sidecar and shows its events while it
/// is the current one (`generation`).
fn watch(app: &AppHandle, window: WebviewWindow, supervisor: Supervisor, generation: u64) {
    // Stored before its events are read, so a Ready finds its child pid.
    let events = {
        let sidecar = app.state::<Sidecar>();
        let mut slot = sidecar.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.insert(supervisor).take_events()
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let pages = app.state::<Arc<Pages>>().inner().clone();
        for event in events {
            let control = pages.bench.lock().unwrap_or_else(|e| e.into_inner());
            if control.current(generation) {
                handle_event(&app, &window, &event);
            }
        }
    });
}

/// The `ORAG_HOME` a serve spec runs with.
fn home_of(spec: &SidecarSpec) -> PathBuf {
    spec.env
        .iter()
        .find(|(key, _)| key == "ORAG_HOME")
        .map(|(_, value)| PathBuf::from(value))
        .unwrap_or_default()
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
    // Leaving the service page: its language (reported when chosen) goes
    // along to the status page (`page_loaded`).
    set(&pages.service, None);
    pages.on_service.store(false, Ordering::SeqCst);
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

/// Acts on a bench command the page shown may give (`bench::channel`),
/// under the lock that event handling takes (`bench::control`).
fn bench_command(app: &AppHandle, pages: &Arc<Pages>, url: &Url) {
    let Some(command) = channel::command(url) else {
        return;
    };
    let shown = if pages.on_local.load(Ordering::SeqCst) {
        Shown::Local
    } else if pages.on_service.load(Ordering::SeqCst) {
        Shown::Service
    } else {
        Shown::Other
    };
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let mut control = pages.bench.lock().unwrap_or_else(|e| e.into_inner());
    match control.command(command, shown) {
        None => {}
        Some(Step::StartBench) => {
            pages.bench_cancel.store(false, Ordering::SeqCst);
            show(
                app,
                &window,
                bench_status(&crate::bench::Progress::default(), false),
            );
            drop(control);
            start_bench(app, pages, window);
        }
        Some(Step::Cancel) => pages.bench_cancel.store(true, Ordering::SeqCst),
        Some(Step::Resume { generation }) => {
            show(app, &window, Status::Loading);
            drop(control);
            let backends = pages
                .paused
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            match backends {
                Some(backends) => watch(app, window, Supervisor::resume(backends), generation),
                None => start_sidecar_as(app, window, generation),
            }
        }
    }
}

/// Runs the bench on a thread of its own: the service is paused there (its
/// stop may take the grace time), before any bench job starts.
fn start_bench(app: &AppHandle, pages: &Arc<Pages>, window: WebviewWindow) {
    let app = app.clone();
    let pages_for_thread = pages.clone();
    let handle = std::thread::spawn(move || {
        let pages = pages_for_thread;
        let supervisor = app
            .state::<Sidecar>()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        set(&pages.paused, supervisor.and_then(Supervisor::pause));
        let result = match (locked(&pages.spec), locked(&pages.data)) {
            (Some(spec), Some(data)) => run_bench(&app, &pages, &window, &spec, data),
            _ => Err(std::io::Error::other("the service was never started")),
        };
        let status = match result {
            Ok(path) => {
                use tauri_plugin_opener::OpenerExt;
                let _ = app.opener().reveal_item_in_dir(&path);
                Status::BenchDone {
                    path: Some(path.display().to_string()),
                    error: None,
                }
            }
            Err(err) => Status::BenchDone {
                path: None,
                error: Some(err.to_string()),
            },
        };
        let mut control = pages.bench.lock().unwrap_or_else(|e| e.into_inner());
        control.finished();
        show(&app, &window, status);
    });
    set(&pages.bench_thread, Some(handle));
}

fn run_bench(
    app: &AppHandle,
    pages: &Arc<Pages>,
    window: &WebviewWindow,
    spec: &SidecarSpec,
    data: PathBuf,
) -> std::io::Result<PathBuf> {
    let vulkan = crate::sidecar::vulkan_beside(&spec.program);
    let bench = crate::bench::Bench {
        cpu: spec.program.clone(),
        vulkan: vulkan.clone().unwrap_or_default(),
        home: home_of(spec),
        data,
        app_version: env!("CARGO_PKG_VERSION").into(),
        reference: crate::bench::reference(),
        timeout: crate::bench::run::JOB_TIMEOUT,
        extra_args: Vec::new(),
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .to_string();
    let cancel = pages.bench_cancel.clone();
    let cancelled = move || cancel.load(Ordering::SeqCst);
    let probe = || match &vulkan {
        Some(program) => {
            let mut probe_spec = spec.clone();
            probe_spec.program = program.clone();
            crate::gpu::run_probe(&probe_spec, crate::gpu::PROBE_TIMEOUT, &cancelled)
        }
        None => crate::gpu::ProbeOutcome::Failed("orag-vulkan-missing".into()),
    };
    let flag = pages.bench_cancel.clone();
    bench.run(&stamp, probe, &cancelled, &mut |progress| {
        show(
            app,
            window,
            bench_status(progress, flag.load(Ordering::SeqCst)),
        );
    })
}

fn bench_status(progress: &crate::bench::Progress, cancelling: bool) -> Status {
    Status::Bench {
        answers_done: progress.answers_done,
        answers_total: progress.answers_total,
        jobs_done: progress.jobs_done,
        jobs_total: progress.jobs_total,
        cancelling,
    }
}

/// On quit: the running job is killed and the bench thread waited for.
fn stop_bench(app: &AppHandle) {
    let pages = app.state::<Arc<Pages>>();
    pages.bench_cancel.store(true, Ordering::SeqCst);
    let handle = pages
        .bench_thread
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}
