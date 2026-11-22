#![windows_subsystem = "windows"]

//! Tricky Injector: a LoadLibrary injector for Minecraft Bedrock (GDK).
//! UI: the system WebView2 via wry; core: Toolhelp process search + CreateRemoteThread.

mod file_dialog;
mod injector;

use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::WindowExtWindows;
use tao::window::{Window, WindowBuilder};
use wry::WebViewBuilder;

const DEFAULT_PROCESS: &str = "Minecraft.Windows.exe";
const WAKE_INTERVAL: Duration = Duration::from_secs(1);
const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const AUTO_INTERVAL: Duration = Duration::from_secs(5);

const HTML_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<meta name="description" content="LoadLibrary injector for Minecraft Bedrock (GDK): pick a DLL in Explorer, inject it manually or let auto-inject check the process every 5 seconds" />
<title>Tricky Injector</title>
<style>
__CSS__
</style>
</head>
<body>
__BODY__
<script>
__JS__
</script>
</body>
</html>
"#;

/// Assembles the single HTML page: fonts and styles are inlined, no external requests.
fn build_html() -> String {
    let css = [
        include_str!("../ui/stamp.css"),
        include_str!("../ui/fonts.css"),
        include_str!("../tokens.css"),
        include_str!("../base.css"),
        include_str!("../ui/style.css"),
    ]
    .concat();
    HTML_TEMPLATE
        .replace("__CSS__", &css)
        .replace("__BODY__", include_str!("../ui/body.html"))
        .replace("__JS__", include_str!("../ui/app.js"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiState<'a> {
    process: String,
    process_default: &'static str,
    dll_name: Option<String>,
    dll_error: Option<String>,
    dll_path: Option<String>,
    auto: bool,
    pids: &'a [u32],
    pending: &'a HashSet<u32>,
}

#[derive(Debug)]
enum UserEvent {
    Command(IpcMsg),
    InjectDone { pid: u32, result: Result<(), String> },
}

#[derive(Debug, Deserialize)]
struct IpcMsg {
    cmd: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
}

impl IpcMsg {
    /// The payload as a string, whatever JSON type the page sent.
    fn value_str(&self) -> Option<String> {
        match self.value.as_ref()? {
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The payload as a bool, accepting both JSON booleans and "true"/"false".
    fn value_bool(&self) -> Option<bool> {
        match self.value.as_ref()? {
            serde_json::Value::Bool(b) => Some(*b),
            serde_json::Value::String(s) => Some(s == "true"),
            _ => None,
        }
    }
}

struct App {
    process_name: String,
    dll_path: Option<PathBuf>,
    dll_error: Option<String>,
    auto: bool,
    last_scan: Instant,
    last_auto: Instant,
    pids: Vec<u32>,
    injected: HashSet<u32>,
    pending: HashSet<u32>,
    dirty: bool,
}

impl App {
    fn new() -> Self {
        Self {
            process_name: DEFAULT_PROCESS.to_string(),
            dll_path: None,
            dll_error: None,
            auto: false,
            last_scan: Instant::now() - SCAN_INTERVAL,
            last_auto: Instant::now(),
            pids: Vec::new(),
            injected: HashSet::new(),
            pending: HashSet::new(),
            dirty: true,
        }
    }

    fn process_name(&self) -> String {
        let n = self.process_name.trim().to_string();
        if n.is_empty() {
            DEFAULT_PROCESS.to_string()
        } else {
            n
        }
    }

    fn sync_pids(&mut self) {
        self.pids = injector::find_process_pids(&self.process_name());
        self.dirty = true;
    }

    /// Injects into every found PID except the already injected ones and the ones in flight.
    fn spawn_injects(&mut self, proxy: &EventLoopProxy<UserEvent>) {
        let Some(dll) = self.dll_path.clone() else { return };
        for pid in self.pids.clone() {
            if self.injected.contains(&pid) || self.pending.contains(&pid) {
                continue;
            }
            self.pending.insert(pid);
            self.dirty = true;
            let proxy = proxy.clone();
            let dll = dll.clone();
            std::thread::spawn(move || {
                let result = injector::inject_dll(pid, &dll);
                let _ = proxy.send_event(UserEvent::InjectDone { pid, result });
            });
        }
    }

    /// The Inject button.
    fn manual_inject(&mut self, proxy: &EventLoopProxy<UserEvent>) {
        if self.dll_path.is_none() {
            return;
        }
        self.sync_pids();
        if !self.pids.is_empty() {
            self.spawn_injects(proxy);
        }
    }

    /// Auto-inject: no more often than once per AUTO_INTERVAL.
    fn auto_tick(&mut self, proxy: &EventLoopProxy<UserEvent>) {
        if !self.auto || self.dll_path.is_none() {
            return;
        }
        if self.last_auto.elapsed() < AUTO_INTERVAL {
            return;
        }
        self.last_auto = Instant::now();
        self.sync_pids();
        self.spawn_injects(proxy);
    }

    fn pick_dll(&mut self, window: &Window) {
        let hwnd = window.hwnd() as *mut c_void;
        match file_dialog::pick_dll_file(hwnd) {
            file_dialog::PickOutcome::Selected(path) => {
                match injector::dll_machine(&path) {
                    Ok(injector::IMAGE_FILE_MACHINE_AMD64) => {
                        self.dll_path = Some(path);
                        self.dll_error = None;
                    }
                    Ok(machine) => {
                        self.dll_error =
                            Some(format!("rejected, not an x64 DLL (machine {machine:#06x})"));
                    }
                    Err(_) => {
                        self.dll_error = Some("rejected, not a valid DLL file".into());
                    }
                }
            }
            file_dialog::PickOutcome::Canceled => {
                self.dll_error = None;
            }
            file_dialog::PickOutcome::Failed(code) => {
                self.dll_error = Some(format!("dialog error {code:#06x}"));
            }
        }
        self.dirty = true;
    }

    fn handle_ipc(&mut self, proxy: &EventLoopProxy<UserEvent>, window: &Window, msg: IpcMsg) {
        match msg.cmd.as_str() {
            "hello" => self.dirty = true,
            "set_process" => {
                if let Some(v) = msg.value_str() {
                    self.process_name = v;
                }
            }
            "pick_dll" => self.pick_dll(window),
            "inject" => self.manual_inject(proxy),
            "set_auto" => {
                let on = msg.value_bool().unwrap_or(false);
                if on != self.auto {
                    self.auto = on;
                    if on {
                        self.last_auto = Instant::now();
                        self.sync_pids();
                        self.spawn_injects(proxy);
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let app = Rc::new(RefCell::new(App::new()));

    let window = WindowBuilder::new()
        .with_title("Tricky Injector")
        .with_inner_size(LogicalSize::new(430.0, 480.0))
        .with_min_inner_size(LogicalSize::new(360.0, 380.0))
        .build(&event_loop)
        .expect("failed to create the main window");

    // The IPC callback only forwards the command into the event loop: no app
    // borrows and no dialogs inside the WebView2 callback, so nothing can be
    // silently dropped on reentrancy.
    let proxy_for_ipc = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(build_html())
        .with_ipc_handler(move |request: wry::http::Request<String>| {
            if let Ok(msg) = serde_json::from_str::<IpcMsg>(request.body()) {
                let _ = proxy_for_ipc.send_event(UserEvent::Command(msg));
            }
        })
        .build(&window)
        .expect("failed to create the WebView");

    let mut next_wake = Instant::now();
    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_wake);

        match event {
            Event::NewEvents(StartCause::Init) => {
                app.borrow_mut().dirty = true;
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                let now = Instant::now();
                if now >= next_wake {
                    next_wake = now + WAKE_INTERVAL;
                    if let Ok(mut app) = app.try_borrow_mut() {
                        if app.last_scan.elapsed() >= SCAN_INTERVAL {
                            app.last_scan = now;
                            app.sync_pids();
                            app.auto_tick(&proxy);
                        }
                    }
                }
            }
            Event::UserEvent(UserEvent::Command(msg)) => {
                if let Ok(mut app) = app.try_borrow_mut() {
                    app.handle_ipc(&proxy, &window, msg);
                }
            }
            Event::UserEvent(UserEvent::InjectDone { pid, result }) => {
                if let Ok(mut app) = app.try_borrow_mut() {
                    app.pending.remove(&pid);
                    if result.is_ok() {
                        app.injected.insert(pid);
                    }
                    app.dirty = true;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
                return;
            }
            _ => {}
        }

        if let Ok(mut app) = app.try_borrow_mut() {
            if app.dirty {
                app.dirty = false;
                let dll_name = app
                    .dll_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().into_owned());
                let state = UiState {
                    process: app.process_name(),
                    process_default: DEFAULT_PROCESS,
                    dll_name,
                    dll_error: app.dll_error.clone(),
                    dll_path: app.dll_path.as_ref().map(|p| p.display().to_string()),
                    auto: app.auto,
                    pids: &app.pids,
                    pending: &app.pending,
                };
                let json = serde_json::to_string(&state).expect("state is always serializable");
                let _ = webview.evaluate_script(&format!("window.__set_state({json});"));
            }
        }
    });
}
