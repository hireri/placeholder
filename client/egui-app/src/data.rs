//! Remote data the pages show: each `Remote` fetches once on a worker thread, then serves the cached result.

use crate::backend;
use eframe::egui::Context;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

pub enum State<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

pub struct Remote<T>(Arc<Mutex<State<T>>>);

impl<T: Send + 'static> Remote<T> {
    pub fn new() -> Self {
        Remote(Arc::new(Mutex::new(State::Idle)))
    }

    pub fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.0.lock().unwrap()
    }

    /// Starts `load` the first time it is called (and again after `reset`).
    pub fn ensure(&self, ctx: &Context, load: impl FnOnce() -> Result<T, String> + Send + 'static) {
        let mut s = self.lock();
        if !matches!(*s, State::Idle) {
            return;
        }
        *s = State::Loading;
        let (slot, ctx) = (self.0.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = load();
            *slot.lock().unwrap() = r.map_or_else(State::Failed, State::Ready);
            ctx.request_repaint();
        });
    }

    pub fn reset(&self) {
        *self.lock() = State::Idle;
    }
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    /// song | album | playlist | artist
    pub r#type: String,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub thumbnail: Option<String>,
    pub duration_sec: Option<u32>,
    /// Browse id of the song's first artist.
    pub artist_id: Option<String>,
}

impl Item {
    /// A song item as a playable track; the subtitle holds the artists.
    pub fn track(&self) -> Track {
        Track {
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.subtitle.as_deref().unwrap_or("").split(", ").filter(|a| !a.is_empty()).map(String::from).collect(),
            album: None,
            duration_sec: self.duration_sec,
            thumbnail: self.thumbnail.clone(),
            source: "youtube".into(),
            artist_id: self.artist_id.clone(),
            album_id: None,
        }
    }
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Section {
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_sec: Option<u32>,
    pub thumbnail: Option<String>,
    /// youtube | spotify
    pub source: String,
    /// Browse id of the first artist (YouTube only).
    pub artist_id: Option<String>,
    /// Browse id of the album (YouTube only).
    pub album_id: Option<String>,
}

impl Track {
    pub fn dur(&self) -> String {
        self.duration_sec.map_or(String::new(), |s| format!("{}:{:02}", s / 60, s % 60))
    }
}

#[derive(Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Detail {
    pub item: Item,
    pub description: Option<String>,
    pub tracks: Vec<Track>,
    pub sections: Vec<Section>,
    /// Artists only: the channel subscriptions go through, whether the user is subscribed, and e.g. "1.2M monthly audience".
    pub channel_id: Option<String>,
    pub subscribed: bool,
    pub monthly_listeners: Option<String>,
}

/// YouTube Music history plus aggregates, computed once on load.
#[derive(Default)]
pub struct Stats {
    pub sections: Vec<Section>,
    pub plays: usize,
    pub songs: usize,
    pub artists: usize,
    pub secs: u32,
    pub top_artists: Vec<(String, usize)>,
    pub top_songs: Vec<(Item, usize)>,
}

impl Stats {
    pub fn new(sections: Vec<Section>) -> Self {
        let mut songs: HashMap<&str, (&Item, usize)> = HashMap::new();
        let mut artists: HashMap<&str, usize> = HashMap::new();
        let (mut plays, mut secs) = (0, 0);
        for it in sections.iter().flat_map(|s| &s.items).filter(|i| i.r#type == "song") {
            plays += 1;
            secs += it.duration_sec.unwrap_or(0);
            songs.entry(&it.id).or_insert((it, 0)).1 += 1;
            for a in it.subtitle.as_deref().unwrap_or("").split(", ").filter(|a| !a.is_empty()) {
                *artists.entry(a).or_default() += 1;
            }
        }
        let mut top_artists: Vec<(String, usize)> = artists.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        top_artists.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        top_artists.truncate(5);
        let (n_songs, n_artists) = (songs.len(), artists.len());
        let mut by_song: Vec<_> = songs.into_values().collect();
        by_song.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.title.cmp(&b.0.title)));
        let top_songs = by_song.into_iter().take(5).map(|(i, n)| (i.clone(), n)).collect();
        Stats { plays, songs: n_songs, artists: n_artists, secs, top_artists, top_songs, sections }
    }
}

pub struct Data {
    pub status: Remote<backend::Status>,
    pub home: Remote<Vec<Section>>,
    pub history: Remote<Stats>,
    /// Number of Spotify playlists (its listening history is not exposed).
    pub spotify: Remote<Vec<Item>>,
    /// Saved playlists, albums and artists for the sidebar, tagged with their filter index (1 playlist, 2 album, 3 artist).
    pub library: Remote<Vec<(usize, Item)>>,
    /// Song search results by query.
    pub searches: Mutex<HashMap<String, Arc<Remote<Vec<Track>>>>>,
    /// Album and artist search results by `type/query`.
    pub item_searches: Mutex<HashMap<String, Arc<Remote<Vec<Item>>>>>,
    /// Error from the last library save, shown on the album page.
    pub notice: Mutex<Option<String>>,
    pub details: Mutex<HashMap<String, Arc<Remote<Detail>>>>,
    /// Set while a sign-in/out is in flight (the login window blocks), with the last error.
    pub busy: Arc<Mutex<(Option<&'static str>, Option<String>)>>,
}

impl Data {
    pub fn new() -> Self {
        Data {
            status: Remote::new(),
            home: Remote::new(),
            history: Remote::new(),
            spotify: Remote::new(),
            library: Remote::new(),
            searches: Default::default(),
            item_searches: Default::default(),
            notice: Default::default(),
            details: Default::default(),
            busy: Default::default(),
        }
    }

    /// `kind` is the endpoint segment: albums | artists | playlists, or `spotify` for an imported playlist.
    pub fn detail(&self, ctx: &Context, kind: &'static str, id: &str) -> Arc<Remote<Detail>> {
        let r = self.details.lock().unwrap().entry(format!("{kind}/{id}")).or_insert_with(|| Arc::new(Remote::new())).clone();
        if kind == "spotify" {
            let path = format!("/spotify/playlists/{id}/tracks");
            let item = match &*self.spotify.lock() {
                State::Ready(v) => v.iter().find(|i| i.id == id).cloned(),
                _ => None,
            };
            let item = item.unwrap_or(Item { title: "Spotify playlist".into(), ..Default::default() });
            r.ensure(ctx, move || Ok(Detail { item, tracks: backend::get(&path)?, ..Default::default() }));
        } else {
            let path = format!("/youtube/{kind}/{id}");
            r.ensure(ctx, move || backend::get(&path));
        }
        r
    }

    pub fn search(&self, ctx: &Context, q: &str) -> Arc<Remote<Vec<Track>>> {
        let r = self.searches.lock().unwrap().entry(q.to_string()).or_insert_with(|| Arc::new(Remote::new())).clone();
        let path = format!("/youtube/search?q={}", backend::enc(q));
        r.ensure(ctx, move || backend::get(&path));
        r
    }

    /// Albums or artists (`kind` is the search filter: album | artist) matching `q`.
    pub fn search_items(&self, ctx: &Context, q: &str, kind: &'static str) -> Arc<Remote<Vec<Item>>> {
        let r = self.item_searches.lock().unwrap().entry(format!("{kind}/{q}")).or_insert_with(|| Arc::new(Remote::new())).clone();
        let path = format!("/youtube/search/items?q={}&type={kind}", backend::enc(q));
        r.ensure(ctx, move || backend::get(&path));
        r
    }

    pub fn load_spotify(&self, ctx: &Context) {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Pl {
            id: String,
            title: String,
            owner: Option<String>,
            thumbnail: Option<String>,
        }
        self.spotify.ensure(ctx, || {
            let v: Vec<Pl> = backend::get("/spotify/playlists")?;
            Ok(v.into_iter().map(|p| Item { r#type: "spotify".into(), id: p.id, title: p.title, subtitle: p.owner, thumbnail: p.thumbnail, ..Default::default() }).collect())
        });
    }

    pub fn load_library(&self, ctx: &Context) {
        self.library.ensure(ctx, || {
            let mut all = vec![];
            for (kind, n) in [("playlists", 1), ("albums", 2), ("artists", 3)] {
                let items: Vec<Item> = backend::get(&format!("/youtube/library/{kind}"))?;
                all.extend(items.into_iter().map(|i| (n, i)));
            }
            Ok(all)
        });
    }

    pub fn reset_content(&self) {
        self.status.reset();
        self.home.reset();
        self.history.reset();
        self.spotify.reset();
        self.library.reset();
        self.searches.lock().unwrap().clear();
        self.item_searches.lock().unwrap().clear();
        self.details.lock().unwrap().clear();
    }

    /// Adds or removes an album (`kind` albums) or playlist from the YouTube Music library; the sidebar reloads on success.
    pub fn save(self: &Arc<Self>, ctx: &Context, kind: &'static str, id: &str, on: bool) {
        let (me, ctx) = (self.clone(), ctx.clone());
        let path = format!("/youtube/save/{kind}/{id}?on={on}");
        *me.notice.lock().unwrap() = None;
        std::thread::spawn(move || {
            let r = backend::post(&path);
            if r.is_ok() {
                me.library.reset();
            }
            *me.notice.lock().unwrap() = r.err();
            ctx.request_repaint();
        });
    }

    /// Subscribes to (or leaves) an artist's channel; on success the cached artist page (`artists/{artist_id}`) is updated in place.
    pub fn subscribe(self: &Arc<Self>, ctx: &Context, artist_id: &str, channel_id: &str, on: bool) {
        let (me, ctx) = (self.clone(), ctx.clone());
        let (path, key) = (format!("/youtube/subscribe/{channel_id}?on={on}"), format!("artists/{artist_id}"));
        *me.notice.lock().unwrap() = None;
        std::thread::spawn(move || {
            let r = backend::post(&path);
            if r.is_ok() {
                let page = me.details.lock().unwrap().get(&key).cloned();
                if let Some(d) = page {
                    if let State::Ready(d) = &mut *d.lock() {
                        d.subscribed = on;
                    }
                }
                me.library.reset();
            }
            *me.notice.lock().unwrap() = r.err();
            ctx.request_repaint();
        });
    }

    /// Runs the blocking sign-in/out on a worker, then drops every cached result so pages refetch.
    pub fn account(self: &Arc<Self>, ctx: &Context, svc: &'static str, sign_in: bool) {
        let (me, ctx) = (self.clone(), ctx.clone());
        *me.busy.lock().unwrap() = (Some(svc), None);
        std::thread::spawn(move || {
            let r = if sign_in { backend::sign_in(svc) } else { backend::sign_out(svc) };
            *me.busy.lock().unwrap() = (None, r.err());
            me.reset_content();
            ctx.request_repaint();
        });
    }
}
