#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod audio;
mod desktop;
mod import;
mod pkg;
mod settings;
mod tex;
mod unpack;
mod we;

use serde_json::{json, Value};
use settings::Settings;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, TrayIconBuilder,
};
use url::Url;
use windows::Win32::Foundation::HWND;
use winit::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtWindows};

// у всех WebView процесса аргументы должны совпадать
const ARGS: &str = "--allow-file-access-from-files --autoplay-policy=no-user-gesture-required";

enum UserEvent {
    Ipc(String),
    Menu(MenuEvent),
    Tick,
    Audio(Vec<f32>),
    Import(Result<String, String>),
}

// webview объявлен раньше window: он должен удаляться первым
struct Wp {
    webview: WebView,
    _window: Window,
}
struct Ui {
    webview: WebView,
    window: Window,
}

fn make_icon() -> Icon {
    let n = 32usize;
    let mut px = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let c = if dx * dx + dy * dy <= 196.0 { [0x5E, 0xEA, 0xD4, 0xFF] } else { [0, 0, 0, 0] };
            px.extend_from_slice(&c);
        }
    }
    Icon::from_rgba(px, n as u32, n as u32).unwrap()
}

/// Одно окно с обоями на каждый монитор, у которого назначены обои.
fn apply(elwt: &ActiveEventLoop, cfg: &Settings) -> Vec<Wp> {
    let mons: Vec<_> = elwt.available_monitors().collect();
    let ox = mons.iter().map(|m| m.position().x).min().unwrap_or(0);
    let oy = mons.iter().map(|m| m.position().y).min().unwrap_or(0);
    let mut out = vec![];
    for (i, m) in mons.iter().enumerate() {
        let Some(path) = cfg.for_monitor(i) else { continue };
        let Ok(loaded) = we::load(Path::new(&path)) else { continue };
        let Ok(window) =
            elwt.create_window(Window::default_attributes().with_decorations(false).with_visible(false))
        else {
            continue;
        };
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(h) = h.as_raw() {
                let _ = desktop::attach(
                    HWND(h.hwnd.get() as _),
                    m.position().x - ox,
                    m.position().y - oy,
                    m.size().width as i32,
                    m.size().height as i32,
                );
            }
        }
        if let Ok(webview) = WebViewBuilder::new()
            .with_url(&loaded.url)
            .with_initialization_script(&loaded.init_js)
            .with_additional_browser_args(ARGS)
            .build(&window)
        {
            out.push(Wp { webview, _window: window });
        }
    }
    out
}

fn build_ui(elwt: &ActiveEventLoop, proxy: EventLoopProxy<UserEvent>, show: bool) -> anyhow::Result<Ui> {
    let page = std::env::temp_dir().join("luma_ui.html");
    std::fs::write(&page, include_str!("ui.html"))?;
    let url = Url::from_file_path(&page).map_err(|_| anyhow::anyhow!("путь"))?;
    let window = elwt.create_window(
        Window::default_attributes()
            .with_title("Luma Wallpaper")
            .with_inner_size(LogicalSize::new(1180.0, 740.0))
            .with_visible(show),
    )?;
    let webview = WebViewBuilder::new()
        .with_url(url.as_str())
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .with_additional_browser_args(ARGS)
        .build(&window)?;
    Ok(Ui { webview, window })
}

fn push(ui: &Ui, cfg: &Settings, monitors: usize) {
    let s = json!({ "items": we::scan(Path::new(&cfg.folder)), "settings": cfg, "monitors": monitors });
    let _ = ui.webview.evaluate_script(&format!("window.luma.state({s})"));
}

fn want_pause(cfg: &Settings, user: bool) -> bool {
    user || (cfg.pause_fullscreen && desktop::fullscreen_busy()) || (cfg.pause_battery && desktop::on_battery())
}

fn set_pause(wps: &[Wp], p: bool) {
    for w in wps {
        let _ = w.webview.evaluate_script(&format!("window.__lumaPause&&window.__lumaPause({p})"));
    }
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("unpack") {
        desktop::attach_console();
        return match (argv.get(2), argv.get(3)) {
            (Some(p), Some(o)) => unpack::run(Path::new(p), Path::new(o)),
            _ => {
                println!("Использование: luma-wallpaper unpack <scene.pkg> <папка>");
                Ok(())
            }
        };
    }
    let tray_only = argv.iter().any(|a| a == "--tray");
    let mut cfg = Settings::load();

    let ev = EventLoop::<UserEvent>::with_user_event().build()?;
    let proxy = ev.create_proxy();

    let menu = Menu::new();
    let open = MenuItem::new("Открыть", true, None);
    let pause = MenuItem::new("Пауза / продолжить", true, None);
    let quit = MenuItem::new("Выход", true, None);
    menu.append(&open)?;
    menu.append(&pause)?;
    menu.append(&quit)?;
    let (open_id, pause_id, quit_id) = (open.id().clone(), pause.id().clone(), quit.id().clone());
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Luma Wallpaper")
        .with_icon(make_icon())
        .build()?;

    let p = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(UserEvent::Menu(e));
    }));
    let p = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        if p.send_event(UserEvent::Tick).is_err() {
            break;
        }
    });

    let audio_on = Arc::new(AtomicBool::new(cfg.audio));
    let p = proxy.clone();
    let _audio = audio::start(audio_on.clone(), move |v| {
        let _ = p.send_event(UserEvent::Audio(v));
    });

    let mut ui: Option<Ui> = None;
    let mut wps: Vec<Wp> = vec![];
    let (mut user_paused, mut paused) = (false, false);

    #[allow(deprecated)]
    ev.run(move |event, elwt| match event {
        Event::NewEvents(StartCause::Init) => {
            ui = build_ui(elwt, proxy.clone(), !tray_only).ok();
            wps = apply(elwt, &cfg);
        }
        Event::WindowEvent { window_id, event: WindowEvent::CloseRequested } => {
            // крестик прячет окно в трей
            if let Some(u) = &ui {
                if u.window.id() == window_id {
                    u.window.set_visible(false);
                }
            }
        }
        Event::UserEvent(UserEvent::Menu(e)) => {
            if e.id == open_id {
                if let Some(u) = &ui {
                    u.window.set_visible(true);
                    u.window.focus_window();
                }
            } else if e.id == pause_id {
                user_paused = !user_paused;
            } else if e.id == quit_id {
                elwt.exit();
            }
        }
        Event::UserEvent(UserEvent::Tick) => {
            let want = want_pause(&cfg, user_paused);
            if want != paused {
                paused = want;
                set_pause(&wps, paused);
            }
        }
        Event::UserEvent(UserEvent::Audio(v)) => {
            if !paused && !wps.is_empty() {
                let js = format!("window.__lumaAudio&&window.__lumaAudio({})", json!(v));
                for w in &wps {
                    let _ = w.webview.evaluate_script(&js);
                }
            }
        }
        Event::UserEvent(UserEvent::Import(r)) => {
            let (ok, msg) = match r {
                Ok(m) => (true, m),
                Err(e) => (false, e),
            };
            if let Some(u) = &ui {
                let _ = u.webview.evaluate_script(&format!("window.luma.toast({},{})", json!(msg), ok));
                push(u, &cfg, elwt.available_monitors().count());
            }
        }
        Event::UserEvent(UserEvent::Ipc(msg)) => {
            let Ok(v) = serde_json::from_str::<Value>(&msg) else { return };
            let mons = elwt.available_monitors().count();
            match v["cmd"].as_str() {
                Some("apply") => {
                    let path = v["path"].as_str().unwrap_or("").to_string();
                    match v["monitor"].as_str() {
                        Some("all") | None => {
                            cfg.all = Some(path);
                            cfg.per_monitor.clear();
                        }
                        Some(i) => {
                            cfg.per_monitor.insert(i.to_string(), path);
                        }
                    }
                    cfg.save();
                    wps = apply(elwt, &cfg);
                    paused = false; // следующий Tick выставит паузу заново
                }
                Some("set") => {
                    let on = v["value"].as_bool().unwrap_or(false);
                    match v["key"].as_str() {
                        Some("autostart") => {
                            cfg.autostart = on;
                            let _ = desktop::set_autostart(on);
                        }
                        Some("pause_fullscreen") => cfg.pause_fullscreen = on,
                        Some("pause_battery") => cfg.pause_battery = on,
                        Some("audio") => {
                            cfg.audio = on;
                            audio_on.store(on, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                    cfg.save();
                }
                Some("import") => {
                    let url = v["url"].as_str().unwrap_or("").trim().to_string();
                    let (c, p) = (cfg.clone(), proxy.clone());
                    std::thread::spawn(move || {
                        let r = import::from_url(&url, &c).map_err(|e| e.to_string());
                        let _ = p.send_event(UserEvent::Import(r));
                    });
                }
                Some("set_text") => {
                    let t = v["value"].as_str().unwrap_or("").trim().to_string();
                    match v["key"].as_str() {
                        Some("steamcmd") => cfg.steamcmd = t,
                        Some("steam_user") => cfg.steam_user = t,
                        _ => {}
                    }
                    cfg.save();
                }
                Some("pick_folder") => {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        cfg.folder = d.to_string_lossy().into();
                        cfg.save();
                    }
                }
                _ => {} // init, rescan
            }
            if let Some(u) = &ui {
                push(u, &cfg, mons);
            }
        }
        _ => {}
    })?;
    Ok(())
}
