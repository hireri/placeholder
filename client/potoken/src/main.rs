//! Hidden webview that runs YouTube's BotGuard page for the daemon, which has no browser of its own.
//! The daemon does the network calls; this only evaluates JS. Usage: serafin-potoken <page.html>
//! stdin:  one JSON per line `{"id":N,"js":"<async function body>"}`; the body's `return` value comes back.
//! stdout: `{"id":N,"ok":bool,"value":"..."}`, plus `{"id":0,"ok":true,"value":"ready"}` once the page has loaded.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::io::BufRead;
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
#[cfg(unix)]
use tao::platform::unix::WindowExtUnix;
#[cfg(unix)]
use wry::WebViewBuilderExtUnix;
use wry::{PageLoadEvent, WebViewBuilder};

enum Ev {
    Eval(u64, String),
    Quit,
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: serafin-potoken <page.html>");
        std::process::exit(2);
    });
    let html = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    });

    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new().with_visible(false).build(&event_loop).unwrap();
    let builder = WebViewBuilder::new()
        .with_html(html)
        .with_ipc_handler(|req| println!("{}", req.body()))
        .with_on_page_load_handler(|ev, _| {
            if matches!(ev, PageLoadEvent::Finished) {
                println!(r#"{{"id":0,"ok":true,"value":"ready"}}"#);
            }
        });
    #[cfg(windows)]
    let webview = builder.build(&window);
    #[cfg(unix)]
    let webview = builder.build_gtk(window.default_vbox().unwrap());
    let webview = webview.unwrap();

    // The daemon owns our lifetime: stdin closing means it is gone.
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            if let (Some(id), Some(js)) = (v["id"].as_u64(), v["js"].as_str()) {
                let _ = proxy.send_event(Ev::Eval(id, js.to_string()));
            }
        }
        let _ = proxy.send_event(Ev::Quit);
    });

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Eval(id, js)) => {
                let script = format!(
                    "(async()=>{{try{{const v=await(async()=>{{{js}\n}})();window.ipc.postMessage(JSON.stringify({{id:{id},ok:true,value:String(v)}}))}}catch(e){{window.ipc.postMessage(JSON.stringify({{id:{id},ok:false,value:String(e)+'\\n'+(e&&e.stack||'')}}))}}}})()"
                );
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(Ev::Quit) => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
