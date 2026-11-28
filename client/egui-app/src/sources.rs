//! State of the Sources settings tab: the daemon's `/sources` list and the background calls that edit it.

use crate::backend;
use eframe::egui::Context;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub enabled: bool,
    pub status: String,
    pub error: Option<String>,
    pub detail: Option<String>,
    pub url: Option<String>,
    pub username: Option<String>,
    pub scrobble_via: String,
    pub caps: Vec<String>,
    /// Shown on Home; a missing field counts as shown.
    #[serde(default = "yes")]
    pub home: bool,
    /// Included in search; a missing field counts as included.
    #[serde(default = "yes")]
    pub search: bool,
    /// YouTube Music: always there, only turned on or off.
    pub builtin: bool,
    /// SoundCloud, Bandcamp: always listed, only turned on or off.
    pub singleton: bool,
    /// A token or password is saved.
    pub secret_set: bool,
    /// The signed-in account's picture (SoundCloud).
    pub avatar_url: Option<String>,
}

fn yes() -> bool {
    true
}

impl Source {
    /// "Connected · detail", "Can't connect: …" and the like, for the row's second line.
    pub fn line(&self, checking: bool) -> String {
        match (self.enabled, checking, self.status.as_str()) {
            (false, ..) => "Turned off".into(),
            (_, true, _) => "Checking…".into(),
            (_, _, "ok") => self.detail.as_deref().filter(|d| !d.is_empty()).map_or("Connected".into(), |d| format!("Connected · {d}")),
            (_, _, "error") => format!("Can't connect: {}", self.error.as_deref().unwrap_or("unknown error")),
            _ => "Not checked yet".into(),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Test {
    ok: bool,
    error: Option<String>,
    detail: Option<String>,
    avatar: Option<String>,
}

#[derive(Default)]
struct Inner {
    list: Option<Result<Vec<Source>, String>>,
    loading: bool,
    checking: HashSet<String>,
    /// The add form's call in flight, its last result (ok, text), and whether an add just went through.
    busy: bool,
    msg: Option<(bool, String)>,
    added: bool,
    /// When a source was last added, removed, reordered, toggled or edited: everything cached from the daemon must be fetched again once it settles.
    changed: Option<Instant>,
    /// Sources with a PUT in flight, and the edits made meanwhile (merged), sent when it returns: requests go out one at a time per source, in order.
    signing: bool,
    sending: HashSet<String>,
    queued: std::collections::HashMap<String, Value>,
}

#[derive(Default)]
pub struct Form {
    pub kind: usize,
    pub name: String,
    pub url: String,
    pub user: String,
    pub secret: String,
    pub via: usize,
}

/// Text buffers of the expanded source.
#[derive(Default)]
pub struct Edit {
    pub name: String,
    pub url: String,
    pub user: String,
    pub secret: String,
}

#[derive(Default)]
pub struct Sources {
    inner: Arc<Mutex<Inner>>,
    /// Expanded source id, or "+" for the add form.
    pub open: Option<String>,
    /// Source whose Remove button was clicked once.
    pub confirm: Option<String>,
    /// Index of the row being dragged.
    pub drag: Option<usize>,
    pub edit: Edit,
    pub form: Form,
}

/// Types the add form offers; SoundCloud and Bandcamp are built in.
pub const TYPES: [&str; 2] = ["navidrome", "jellyfin"];

impl Sources {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// The list, fetched on first use (`None` while it loads); enabled sources are tested right after.
    pub fn list(&self, ctx: &Context) -> Option<Result<Vec<Source>, String>> {
        let mut g = self.lock();
        if g.list.is_none() && !g.loading {
            g.loading = true;
            let (inner, ctx) = (self.inner.clone(), ctx.clone());
            std::thread::spawn(move || {
                let r = backend::get::<Vec<Source>>("/sources");
                let ids: Vec<String> = r.iter().flatten().filter(|s| s.enabled).map(|s| s.id.clone()).collect();
                {
                    let mut g = inner.lock().unwrap();
                    (g.loading, g.list) = (false, Some(r));
                    g.checking.extend(ids.iter().cloned());
                }
                ctx.request_repaint();
                for id in ids {
                    let (inner, ctx) = (inner.clone(), ctx.clone());
                    std::thread::spawn(move || check(&inner, &ctx, &id));
                }
            });
        }
        g.list.clone()
    }

    /// Signing in or out of SoundCloud (the login window is open, or the daemon is being told).
    pub fn signing(&self) -> bool {
        self.lock().signing
    }

    /// Opens SoundCloud's sign-in window, or signs out; the list is fetched again afterwards so the card shows who is signed in.
    pub fn soundcloud_auth(&self, ctx: &Context, sign_in: bool) {
        {
            let mut g = self.lock();
            if g.signing {
                return;
            }
            g.signing = true;
        }
        let (inner, ctx) = (self.inner.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = if sign_in { backend::sign_in("soundcloud") } else { backend::sign_out("soundcloud") };
            match r {
                Ok(()) => crate::toast::info(if sign_in { "Signed in to SoundCloud" } else { "Signed out of SoundCloud" }),
                Err(e) if e == "sign-in cancelled" => {}
                Err(e) => crate::toast::error(e),
            }
            let mut g = inner.lock().unwrap();
            g.signing = false;
            (g.list, g.loading, g.changed) = (None, false, Some(Instant::now()));
            drop(g);
            ctx.request_repaint();
        });
    }

    pub fn checking(&self, id: &str) -> bool {
        self.lock().checking.contains(id)
    }

    pub fn test(&self, ctx: &Context, id: &str) {
        self.lock().checking.insert(id.into());
        let (inner, ctx, id) = (self.inner.clone(), ctx.clone(), id.to_string());
        std::thread::spawn(move || check(&inner, &ctx, &id));
    }

    /// Applies `edit` to the source at once and sends `body`; the reply replaces it, and a changed connection is re-tested.
    pub fn patch(&self, ctx: &Context, id: &str, body: Value, retest: bool, edit: impl FnOnce(&mut Source)) {
        let mut g = self.lock();
        if let Some(s) = g.list.as_mut().and_then(|l| l.as_mut().ok()).and_then(|l| l.iter_mut().find(|s| s.id == id)) {
            edit(s);
        }
        g.changed = Some(Instant::now()); // the settle timer starts at the click, not at the reply
        if g.sending.contains(id) {
            match (g.queued.get_mut(id), body) {
                (Some(Value::Object(q)), Value::Object(b)) => q.extend(b),
                (_, b) => {
                    g.queued.insert(id.into(), b);
                }
            }
            return;
        }
        g.sending.insert(id.into());
        drop(g);
        let (inner, ctx, id) = (self.inner.clone(), ctx.clone(), id.to_string());
        std::thread::spawn(move || {
            let mut body = body;
            loop {
                let r = backend::put_json::<_, Source>(&format!("/sources/{id}"), &body);
                let mut g = inner.lock().unwrap();
                match r {
                    Ok(s) => {
                        let next = g.queued.remove(&id);
                        if next.is_none() {
                            // Nothing newer was asked for, so the reply is the truth.
                            if let Some(Ok(l)) = &mut g.list {
                                l.iter_mut().filter(|o| o.id == id).for_each(|o| *o = s.clone());
                            }
                            g.sending.remove(&id);
                            g.changed = Some(Instant::now());
                            if retest && s.enabled {
                                g.checking.insert(id.clone());
                                drop(g);
                                ctx.request_repaint();
                                check(&inner, &ctx, &id);
                            }
                            ctx.request_repaint();
                            return;
                        }
                        body = next.unwrap();
                    }
                    Err(e) => {
                        (g.sending.remove(&id), g.queued.remove(&id));
                        drop(g);
                        return fail(&inner, &ctx, e);
                    }
                }
            }
        });
    }

    /// Moves row `from` to `to` and tells the daemon the new priority order.
    pub fn reorder(&self, ctx: &Context, from: usize, to: usize) {
        let mut g = self.lock();
        let Some(Ok(l)) = &mut g.list else { return };
        if from >= l.len() || from == to {
            return;
        }
        let s = l.remove(from);
        l.insert(to.min(l.len()), s);
        let ids: Vec<String> = l.iter().map(|s| s.id.clone()).collect();
        drop(g);
        let (inner, ctx) = (self.inner.clone(), ctx.clone());
        std::thread::spawn(move || match backend::put_json::<_, Vec<Source>>("/sources/order", &ids) {
            Ok(l) => {
                with_list(&inner, |cur| *cur = l);
                inner.lock().unwrap().changed = Some(Instant::now());
                ctx.request_repaint();
            }
            Err(e) => fail(&inner, &ctx, e),
        });
    }

    pub fn remove(&self, ctx: &Context, id: &str) {
        let (inner, ctx, id) = (self.inner.clone(), ctx.clone(), id.to_string());
        std::thread::spawn(move || match backend::delete(&format!("/sources/{id}")) {
            Ok(()) => {
                with_list(&inner, |l| l.retain(|s| s.id != id));
                inner.lock().unwrap().changed = Some(Instant::now());
                ctx.request_repaint();
            }
            Err(e) => fail(&inner, &ctx, e),
        });
    }

    /// The add form's request body.
    pub fn form_body(&self) -> Value {
        let f = &self.form;
        let (url, user, secret) = (f.url.trim(), f.user.trim(), f.secret.trim());
        let (kind, via) = (TYPES[f.kind.min(1)], ["serafin", "server"][f.via.min(1)]);
        json!({ "type": kind, "name": f.name.trim(), "url": url, "username": user, "secret": secret, "scrobbleVia": via })
    }

    /// Dry-run test of the form, or (with `add`) the real add; the result lands in `msg`.
    pub fn submit(&self, ctx: &Context, add: bool) {
        let body = self.form_body();
        {
            let mut g = self.lock();
            if g.busy {
                return;
            }
            (g.busy, g.msg) = (true, None);
        }
        let (inner, ctx) = (self.inner.clone(), ctx.clone());
        std::thread::spawn(move || {
            let msg = if add {
                match backend::post_json::<_, Source>("/sources", &body) {
                    Ok(s) => {
                        let mut g = inner.lock().unwrap();
                        if let Some(Ok(l)) = &mut g.list {
                            l.push(s);
                        }
                        (g.added, g.changed) = (true, Some(Instant::now()));
                        None
                    }
                    Err(e) => Some((false, e)),
                }
            } else {
                Some(match backend::post_json::<_, Test>("/sources/test", &body) {
                    Ok(t) if t.ok => (true, t.detail.filter(|d| !d.is_empty()).map_or("Connected".into(), |d| format!("Connected · {d}"))),
                    Ok(t) => (false, format!("Can't connect: {}", t.error.unwrap_or_default())),
                    Err(e) => (false, e),
                })
            };
            let mut g = inner.lock().unwrap();
            (g.busy, g.msg) = (false, msg);
            drop(g);
            ctx.request_repaint();
        });
    }

    /// (busy, last result) of the add form.
    pub fn form_state(&self) -> (bool, Option<(bool, String)>) {
        let g = self.lock();
        (g.busy, g.msg.clone())
    }

    pub fn clear_msg(&self) {
        self.lock().msg = None;
    }

    /// True once, 700 ms after the last change to the sources (toggles in a row count as one).
    pub fn take_changed(&self, ctx: &Context) -> bool {
        const SETTLE: Duration = Duration::from_millis(700);
        let mut g = self.lock();
        let Some(at) = g.changed else { return false };
        match SETTLE.checked_sub(at.elapsed()) {
            Some(left) => {
                ctx.request_repaint_after(left);
                false
            }
            None => {
                g.changed = None;
                true
            }
        }
    }

    /// More than one source is enabled, so every item says where it comes from.
    pub fn multi(&self) -> bool {
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).is_some_and(|l| l.iter().filter(|s| s.enabled).count() > 1)
    }

    pub fn name(&self, src: &str) -> String {
        if matches!(src, "yt" | "youtube") {
            return "YouTube Music".into();
        }
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).and_then(|l| l.iter().find(|s| s.id == src)).map_or_else(|| src.to_string(), |s| s.name.clone())
    }

    /// (id, reason) for each source that is turned off; its songs stay queued but cannot play.
    /// An unreachable source is not listed: its test result can be stale, and a song that fails to load already reports itself.
    pub fn off_list(&self) -> Vec<(String, String)> {
        let g = self.lock();
        let Some(Ok(l)) = &g.list else { return vec![] };
        l.iter().filter(|s| !s.enabled).map(|s| (s.id.clone(), format!("{} is turned off", s.name))).collect()
    }

    /// Whether the source can do `cap` (playlist_write, image, ...).
    pub fn can(&self, src: &str, cap: &str) -> bool {
        let id = if src == "youtube" { "yt" } else { src };
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).is_some_and(|l| l.iter().any(|s| s.id == id && s.enabled && s.caps.iter().any(|c| c == cap)))
    }

    /// Ids of the enabled sources that can do `cap`.
    pub fn with_cap(&self, cap: &str) -> Vec<String> {
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).map(|l| l.iter().filter(|s| s.enabled && s.caps.iter().any(|c| c == cap)).map(|s| s.id.clone()).collect()).unwrap_or_default()
    }

    /// The source scrobbles to Last.fm itself, so Serafin must not send the same play again.
    pub fn server_scrobbles(&self, src: &str) -> bool {
        let id = if src == "youtube" { "yt" } else { src };
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).is_some_and(|l| l.iter().any(|s| s.id == id && s.scrobble_via == "server"))
    }

    /// (id, name) of every source, for labelling.
    pub fn names(&self) -> Vec<(String, String)> {
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).map(|l| l.iter().map(|s| (s.id.clone(), s.name.clone())).collect()).unwrap_or_default()
    }

    /// Names of the enabled sources, in priority order.
    pub fn enabled_names(&self) -> Vec<String> {
        self.lock().list.as_ref().and_then(|l| l.as_ref().ok()).map(|l| l.iter().filter(|s| s.enabled).map(|s| s.name.clone()).collect()).unwrap_or_default()
    }

    pub fn take_added(&self) -> bool {
        std::mem::take(&mut self.lock().added)
    }
}

fn with_list(inner: &Mutex<Inner>, f: impl FnOnce(&mut Vec<Source>)) {
    if let Some(Ok(l)) = &mut inner.lock().unwrap().list {
        f(l);
    }
}

/// Shows the error and refetches the list, so the page shows what the daemon really has.
fn fail(inner: &Mutex<Inner>, ctx: &Context, e: String) {
    crate::toast::error(format!("Couldn't change the source: {e}"));
    inner.lock().unwrap().list = None;
    ctx.request_repaint();
}

/// Blocking: tests one source and writes the outcome into its list entry.
fn check(inner: &Mutex<Inner>, ctx: &Context, id: &str) {
    let r = backend::post_json::<_, Test>(&format!("/sources/{id}/test"), &json!({}));
    let mut g = inner.lock().unwrap();
    g.checking.remove(id);
    if let Some(Ok(l)) = &mut g.list {
        if let Some(s) = l.iter_mut().find(|s| s.id == id) {
            let t = r.unwrap_or_else(|e| Test { ok: false, error: Some(e), ..Default::default() });
            s.status = if t.ok { "ok" } else { "error" }.into();
            (s.error, s.detail) = (t.error, t.detail);
            s.avatar_url = t.avatar;
        }
    }
    drop(g);
    ctx.request_repaint();
}

/// The ids (`youtube` counts as `yt`) name more than one source.
pub fn mixed<'a>(ids: impl Iterator<Item = &'a str>) -> bool {
    let mut ids = ids.map(|s| if s == "youtube" { "yt" } else { s });
    ids.next().is_some_and(|f| ids.any(|s| s != f))
}

#[cfg(test)]
mod tests {
    use super::mixed;

    #[test]
    fn mixed_sources() {
        assert!(!mixed([].into_iter()));
        assert!(!mixed(["yt", "youtube", "yt"].into_iter()));
        assert!(mixed(["yt", "jellyfin-1"].into_iter()));
        assert!(!mixed(["jellyfin-1", "jellyfin-1"].into_iter()));
    }
}
