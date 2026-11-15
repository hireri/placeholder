//! Login popup: opens the service's own sign-in page in a wry webview, waits until the session
//! exists, then hands the credentials straight to the daemon (they never pass through the UI).
//! Usage: metrolist-login <youtube|spotify>   env: METROLIST_URL, METROLIST_TOKEN, METROLIST_DATA

use std::path::PathBuf;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::unix::WindowExtUnix,
    window::WindowBuilder,
};
use wry::{PageLoadEvent, WebContext, WebViewBuilder, WebViewBuilderExtUnix};

const YT_START: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
const SP_START: &str = "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F";
const YT_JS: &str = "JSON.stringify({v:(window.ytcfg&&ytcfg.get('VISITOR_DATA'))||null,d:(window.ytcfg&&ytcfg.get('DATASYNC_ID'))||null,l:(window.ytcfg&&ytcfg.get('LOGGED_IN'))||false})";

enum Ev {
    Loaded(String),
    YtScript(String),
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
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/metrolist-desktop")
    });
    base.join("webview").join(service)
}

fn main() {
    let service = std::env::args().nth(1).unwrap_or_default();
    let (start, host) = match service.as_str() {
        "youtube" => (YT_START, "https://music.youtube.com"),
        "spotify" => (SP_START, "https://open.spotify.com"),
        _ => {
            eprintln!("usage: metrolist-login <youtube|spotify>");
            std::process::exit(2);
        }
    };

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title(if service == "youtube" { "Sign in to YouTube Music" } else { "Sign in to Spotify" })
        .with_inner_size(tao::dpi::LogicalSize::new(480.0, 720.0))
        .build(&event_loop)
        .unwrap();
    let mut ctx = WebContext::new(Some(data_dir(&service)));
    let p = proxy.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(start)
        .with_on_page_load_handler(move |ev, url| {
            if matches!(ev, PageLoadEvent::Finished) {
                let _ = p.send_event(Ev::Loaded(url));
            }
        })
        .build_gtk(window.default_vbox().unwrap())
        .unwrap();

    let mut ok = false;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Loaded(url)) if url.starts_with(host) => {
                if service == "youtube" {
                    let p = proxy.clone();
                    let _ = webview.evaluate_script_with_callback(YT_JS, move |r| {
                        let _ = p.send_event(Ev::YtScript(r));
                    });
                } else if let Some(c) = webview.cookies_for_url(host).unwrap_or_default().iter().find(|c| c.name() == "sp_dc") {
                    match post("/auth/spotify", serde_json::json!({ "spDc": c.value() })) {
                        Ok(()) => { ok = true; *flow = ControlFlow::Exit }
                        Err(e) => eprintln!("daemon rejected credentials: {e}"),
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
