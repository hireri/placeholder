//! Login popup: opens the service's own sign-in page in a wry webview, waits until the session
//! exists, then hands the credentials straight to the daemon (they never pass through the UI).
//! Usage: serafin-login <youtube|spotify|soundcloud>   env: METROLIST_URL, METROLIST_TOKEN, METROLIST_DATA
//! Also, for the daemon: serafin-login soundcloud-like <on|off> <user id> <track id>   env: SC_TOKEN, SC_CLIENT_ID, METROLIST_DATA
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
#[cfg(unix)]
use tao::platform::unix::WindowExtUnix;
#[cfg(unix)]
use wry::WebViewBuilderExtUnix;
use wry::{PageLoadEvent, WebContext, WebViewBuilder};

const YT_START: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
const SP_START: &str = "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F";
const SC_START: &str = "https://soundcloud.com/signin";
const YT_JS: &str = "JSON.stringify({v:(window.ytcfg&&ytcfg.get('VISITOR_DATA'))||null,d:(window.ytcfg&&ytcfg.get('DATASYNC_ID'))||null,l:(window.ytcfg&&ytcfg.get('LOGGED_IN'))||false})";

enum Ev {
    Tick,
    Loaded(String),
    YtScript(String),
    Reply(String),
}

fn post(path: &str, body: serde_json::Value) -> Result<(), String> {
    let base = std::env::var("METROLIST_URL").map_err(|_| "METROLIST_URL unset")?;
    let token = std::env::var("METROLIST_TOKEN").map_err(|_| "METROLIST_TOKEN unset")?;
    ureq::post(&format!("{base}{path}"))
        .set("X-Metrolist-Token", &token)
        .send_json(body)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Where the webview keeps its cookies; the app deletes this directory on sign-out.
fn data_dir(service: &str) -> PathBuf {
    let base = std::env::var("METROLIST_DATA").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default()).join(".config/serafin")
    });
    base.join("webview").join(service)
}

/// Tells the window manager this is a small dialog, not a tiled app window: a fixed size (what Wayland compositors key on) plus the dialog hint (X11 window managers).
fn dialog(window: &tao::window::Window) {
    #[cfg(unix)]
    gtk::prelude::GtkWindowExt::set_type_hint(window.gtk_window(), gtk::gdk::WindowTypeHint::Dialog);
    #[cfg(not(unix))]
    let _ = window;
}

/// Sends a SoundCloud like or unlike from a real page in the saved SoundCloud session: its bot protection (DataDome) answers plain HTTP clients with a captcha.
/// The window stays hidden unless DataDome asks for one; the captcha then shows in an iframe over the page (where DataDome's own script handles the
/// result) while the request is retried every few seconds. Exits 0 once SoundCloud accepts.
fn soundcloud_like(args: &[String]) -> ! {
    let (method, user, track) = (if args[0] == "on" { "PUT" } else { "DELETE" }, &args[1], &args[2]);
    let url = format!(
        "https://api-v2.soundcloud.com/users/{user}/track_likes/{track}?client_id={}",
        std::env::var("SC_CLIENT_ID").unwrap_or_default()
    );
    let auth = serde_json::to_string(&format!("OAuth {}", std::env::var("SC_TOKEN").unwrap_or_default())).unwrap();
    let url_js = serde_json::to_string(&url).unwrap();
    // DataDome's script, loaded with the page, hooks fetch and adds its own headers; the first attempt waits a moment for it.
    let send = move |delay: u32| format!(
        "(async()=>{{try{{await new Promise(r=>setTimeout(r,{delay}));const r=await fetch({url_js},{{method:{method:?},headers:{{Authorization:{auth}}},credentials:'include'}});window.ipc.postMessage(JSON.stringify({{s:r.status,b:(await r.text()).slice(0,2000)}}))}}catch(e){{window.ipc.postMessage(JSON.stringify({{s:0,b:String(e)}}))}}}})()"
    );

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new().with_title("SoundCloud").with_visible(false).with_inner_size(tao::dpi::LogicalSize::new(480.0, 720.0)).with_resizable(false).build(&event_loop).unwrap();
    dialog(&window);
    let mut ctx = WebContext::new(Some(data_dir("soundcloud")));
    let (p, p2) = (proxy.clone(), proxy.clone());
    let builder = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url("https://soundcloud.com/")
        .with_ipc_handler(move |req| {
            let _ = p.send_event(Ev::Reply(req.body().clone()));
        })
        .with_on_page_load_handler(move |ev, url| {
            if matches!(ev, PageLoadEvent::Finished) {
                let _ = p2.send_event(Ev::Loaded(url));
            }
        });
    #[cfg(windows)]
    let webview = builder.build(&window);
    #[cfg(unix)]
    let webview = builder.build_gtk(window.default_vbox().unwrap());
    let webview = webview.unwrap();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if proxy.send_event(Ev::Tick).is_err() {
            break;
        }
    });

    let (mut code, mut waited, mut captcha) = (1, 0u32, false);
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Loaded(url)) if url.starts_with("https://soundcloud.com") => {
                waited = 0;
                let _ = webview.evaluate_script(&send(1500));
            }
            Event::UserEvent(Ev::Reply(body)) => {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let (status, text) = (v["s"].as_u64().unwrap_or(0), v["b"].as_str().unwrap_or_default());
                let challenge = serde_json::from_str::<serde_json::Value>(text).ok().and_then(|j| j["url"].as_str().filter(|u| u.contains("captcha-delivery.com")).map(str::to_string));
                match (status, challenge) {
                    (200..=299, _) => { code = 0; *flow = ControlFlow::Exit }
                    (403, Some(u)) if !captcha => {
                        captcha = true;
                        waited = 0;
                        window.set_visible(true);
                        let frame = format!(
                            "(()=>{{const f=document.createElement('iframe');f.src={};f.allow='accelerometer; gyroscope; magnetometer; clipboard-write';f.style.cssText='position:fixed;inset:0;width:100%;height:100%;border:0;background:#fff;z-index:2147483647';document.body.appendChild(f)}})()",
                            serde_json::to_string(&u).unwrap()
                        );
                        let _ = webview.evaluate_script(&frame);
                    }
                    (403, Some(_)) => {} // still unsolved: the retry below asks again
                    _ => { println!("HTTP {status} {text}"); *flow = ControlFlow::Exit }
                }
            }
            Event::UserEvent(Ev::Tick) => {
                waited += 1;
                if captcha && waited % 3 == 0 {
                    let _ = webview.evaluate_script(&send(0));
                }
                if waited > if captcha { 180 } else { 30 } {
                    println!("{}", if captcha { "captcha not solved" } else { "timed out" });
                    *flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            Event::LoopDestroyed => std::process::exit(code),
            _ => {}
        }
    });
}

fn main() {
    let service = std::env::args().nth(1).unwrap_or_default();
    if service == "soundcloud-like" {
        let args: Vec<String> = std::env::args().skip(2).collect();
        if args.len() == 3 {
            soundcloud_like(&args);
        }
        eprintln!("usage: serafin-login soundcloud-like <on|off> <user id> <track id>");
        std::process::exit(2);
    }
    let (start, host) = match service.as_str() {
        "youtube" => (YT_START, "https://music.youtube.com"),
        "spotify" => (SP_START, "https://open.spotify.com"),
        "soundcloud" => (SC_START, "https://soundcloud.com"),
        _ => {
            eprintln!("usage: serafin-login <youtube|spotify|soundcloud>");
            std::process::exit(2);
        }
    };

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title(match service.as_str() { "youtube" => "Sign in to YouTube Music", "soundcloud" => "Sign in to SoundCloud", _ => "Sign in to Spotify" })
        .with_inner_size(tao::dpi::LogicalSize::new(480.0, 720.0))
        .with_resizable(false)
        .build(&event_loop)
        .unwrap();
    dialog(&window);
    let mut ctx = WebContext::new(Some(data_dir(&service)));
    let p = proxy.clone();
    let builder = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(start)
        .with_on_page_load_handler(move |ev, url| {
            if matches!(ev, PageLoadEvent::Finished) {
                let _ = p.send_event(Ev::Loaded(url));
            }
        });
    #[cfg(windows)]
    let webview = builder.build(&window);
    #[cfg(unix)]
    let webview = builder.build_gtk(window.default_vbox().unwrap());
    let webview = webview.unwrap();

    // SoundCloud is a single-page app: signing in changes the page without a load event, so look for the cookie on a timer too.
    if service == "soundcloud" {
        let p = proxy.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            if p.send_event(Ev::Tick).is_err() {
                break;
            }
        });
    }
    let (mut ok, mut ticks) = (false, 0u32);
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Loaded(url)) if url.starts_with(host) => {
                if service == "youtube" {
                    let p = proxy.clone();
                    let _ = webview.evaluate_script_with_callback(YT_JS, move |r| {
                        let _ = p.send_event(Ev::YtScript(r));
                    });
                } else if service == "soundcloud" {
                    if let Some(c) = webview.cookies_for_url(host).unwrap_or_default().iter().find(|c| c.name() == "oauth_token") {
                        match post("/auth/soundcloud", serde_json::json!({ "token": c.value() })) {
                            Ok(()) => { ok = true; *flow = ControlFlow::Exit }
                            Err(e) => eprintln!("daemon rejected credentials: {e}"),
                        }
                    }
                } else if let Some(c) = webview.cookies_for_url(host).unwrap_or_default().iter().find(|c| c.name() == "sp_dc") {
                    match post("/auth/spotify", serde_json::json!({ "spDc": c.value() })) {
                        Ok(()) => { ok = true; *flow = ControlFlow::Exit }
                        Err(e) => eprintln!("daemon rejected credentials: {e}"),
                    }
                }
            }
            Event::UserEvent(Ev::Tick) if !ok => {
                let all = webview.cookies().unwrap_or_default();
                if let Some(c) = all.iter().find(|c| c.name() == "oauth_token" && c.domain().is_none_or(|d| d.contains("soundcloud.com"))) {
                    match post("/auth/soundcloud", serde_json::json!({ "token": c.value() })) {
                        Ok(()) => { ok = true; *flow = ControlFlow::Exit }
                        Err(e) => eprintln!("daemon rejected credentials: {e}"),
                    }
                } else {
                    ticks += 1;
                    if ticks % 10 == 0 {
                        let names: Vec<String> = all.iter().map(|c| format!("{}@{}", c.name(), c.domain().unwrap_or("?"))).collect();
                        eprintln!("soundcloud login: no oauth_token yet; cookies: {}", names.join(", "));
                    }
                }
            }
            Event::UserEvent(Ev::YtScript(raw)) => {
                let inner: String = serde_json::from_str(&raw).unwrap_or_default();
                let v: serde_json::Value = serde_json::from_str(&inner).unwrap_or_default();
                let cookies = webview.cookies_for_url(host).unwrap_or_default();
                let visitor = v["v"].as_str().filter(|s| !s.is_empty());
                if v["l"].as_bool().unwrap_or(false) && cookies.iter().any(|c| c.name() == "SAPISID") && visitor.is_some() {
                    let header = cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; ");
                    match post("/auth/youtube", serde_json::json!({ "cookie": header, "visitorData": visitor, "dataSyncId": v["d"] })) {
                        Ok(()) => { ok = true; *flow = ControlFlow::Exit }
                        Err(e) => eprintln!("daemon rejected credentials: {e}"),
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            Event::LoopDestroyed => std::process::exit(if ok { 0 } else { 1 }),
            _ => {}
        }
    });
}
