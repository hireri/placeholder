//! Remote data the pages show: each `Remote` fetches once on a worker thread, then serves the cached result.

use crate::backend;
use eframe::egui::Context;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub enum State<T> {
    Idle,
    Loading,
    Ready(Arc<T>),
    Failed(String),
}

pub struct Remote<T>(Arc<Mutex<State<T>>>);

impl<T: Send + Sync + 'static> Remote<T> {
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
            *slot.lock().unwrap() = r.map_or_else(State::Failed, |v| State::Ready(Arc::new(v)));
            ctx.request_repaint();
        });
    }

    /// The loaded value, cloned out so the caller can draw without holding the lock (other code may need it meanwhile).
    pub fn ready(&self) -> Option<Arc<T>> {
        match &*self.lock() {
            State::Ready(v) => Some(v.clone()),
            _ => None,
        }
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
    /// Songs only: the album they sit on (`None` for singles).
    pub album: Option<String>,
    pub album_id: Option<String>,
    /// Songs only: marked explicit / a music video rather than the studio track.
    pub explicit: bool,
    pub video: bool,
    /// Playlists only: owned by the user, so it can be deleted.
    pub editable: bool,
}

/// Content filters from the settings page (upstream `HideExplicitKey` / `HideVideoSongsKey`); applied as responses arrive.
pub static HIDE_EXPLICIT: AtomicBool = AtomicBool::new(false);
pub static HIDE_VIDEO: AtomicBool = AtomicBool::new(false);

fn hidden(explicit: bool, video: bool) -> bool {
    explicit && HIDE_EXPLICIT.load(Ordering::Relaxed) || video && HIDE_VIDEO.load(Ordering::Relaxed)
}

/// Home shelves the user hid, as `HOME_*` bits; applied to the feed as it arrives (shelf titles are all the daemon gives).
pub static HOME_HIDE: AtomicU8 = AtomicU8::new(0);
pub const HOME_SHELVES: [(u8, &[&str]); 4] = [(1, &["listen again", "recent"]), (2, &["mood", "genre"]), (4, &["quick picks", "top picks"]), (8, &["short"])];
pub const HOME_MUSIC_ONLY: u8 = 16;

pub fn clean_home(v: &mut Vec<Section>) {
    let m = HOME_HIDE.load(Ordering::Relaxed);
    v.retain(|s| {
        let t = s.title.to_lowercase();
        !HOME_SHELVES.iter().any(|(bit, words)| m & bit != 0 && words.iter().any(|w| t.contains(w))) && (m & HOME_MUSIC_ONLY == 0 || !["podcast", "episode"].iter().any(|w| t.contains(w)))
    });
    if m & HOME_MUSIC_ONLY != 0 {
        v.iter_mut().for_each(|s| s.items.retain(|i| !i.video));
        v.retain(|s| !s.items.is_empty());
    }
}

pub fn clean_sections(v: &mut Vec<Section>) {
    for s in v.iter_mut() {
        s.items.retain(|i| !hidden(i.explicit, i.video));
    }
    v.retain(|s| !s.items.is_empty());
}

impl Item {
    /// A song item as a playable track; the subtitle holds the artists.
    pub fn track(&self) -> Track {
        Track {
            id: self.id.clone(),
            title: self.title.clone(),
            artists: self.subtitle.as_deref().unwrap_or("").split(", ").filter(|a| !a.is_empty()).map(String::from).collect(),
            album: self.album.clone(),
            duration_sec: self.duration_sec,
            thumbnail: self.thumbnail.clone(),
            source: "youtube".into(),
            artist_id: self.artist_id.clone(),
            album_id: self.album_id.clone(),
            ..Default::default()
        }
    }
}

#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Section {
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
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
    pub explicit: bool,
    pub video: bool,
    /// Spotify import only: the YouTube counterpart is still being looked up.
    #[serde(default)]
    pub resolving: bool,
    /// Spotify import only: no YouTube counterpart was found; the row keeps its Spotify metadata.
    #[serde(default)]
    pub unavailable: bool,
}

impl Track {
    pub fn dur(&self) -> String {
        self.duration_sec.map_or(String::new(), |s| format!("{}:{:02}", s / 60, s % 60))
    }
}

/// The daemon's answer to `/youtube/match`: the matched track, or none.
#[derive(Deserialize, Default)]
struct Match {
    track: Option<Track>,
    /// The daemon's search failed (rate limit, network); the row should stay retryable rather than look unmatched.
    #[serde(default)]
    retry: bool,
}

#[derive(serde::Serialize)]
struct ExportReq<'a> {
    title: &'a str,
    #[serde(rename = "videoIds")]
    video_ids: &'a [String],
}

#[derive(Deserialize)]
struct ExportRes {
    id: String,
    added: usize,
}

struct Wanted {
    detail: Arc<Remote<Detail>>,
    ctx: Context,
    wanted: BTreeSet<usize>,
    active: HashSet<usize>,
    seen: Instant,
}

/// Resolves only the Spotify rows currently near the viewport. The UI replaces each playlist's wanted range as
/// it scrolls, so a fast fling never leaves a queue of off-screen lookups behind.
struct SpotifyMatcher {
    windows: Mutex<HashMap<String, Wanted>>,
    wake: Condvar,
    /// Lookups finished since the titlebar island last collected them.
    done: AtomicUsize,
}

impl SpotifyMatcher {
    fn start() -> Arc<Self> {
        let this = Arc::new(Self { windows: Default::default(), wake: Condvar::new(), done: AtomicUsize::new(0) });
        for _ in 0..2 {
            let this = this.clone();
            std::thread::spawn(move || this.work());
        }
        this
    }

    fn request(&self, ctx: &Context, id: &str, detail: &Arc<Remote<Detail>>, range: Range<usize>) {
        let mut desired = BTreeSet::new();
        if let State::Ready(d) = &mut *detail.lock() {
            let d = Arc::make_mut(d);
            for i in range {
                let Some(t) = d.tracks.get_mut(i) else { continue };
                if t.source != "spotify" || t.unavailable {
                    continue;
                }
                if !t.resolving {
                    t.resolving = true;
                    ctx.request_repaint();
                }
                desired.insert(i);
            }
        }
        let mut windows = self.windows.lock().unwrap();
        let w = windows.entry(id.to_string()).or_insert_with(|| Wanted {
            detail: detail.clone(),
            ctx: ctx.clone(),
            wanted: BTreeSet::new(),
            active: HashSet::new(),
            seen: Instant::now(),
        });
        w.detail = detail.clone();
        w.ctx = ctx.clone();
        w.seen = Instant::now();
        w.wanted = desired.into_iter().filter(|i| !w.active.contains(i)).collect();
        drop(windows);
        self.wake.notify_all();
    }

    fn work(&self) {
        loop {
            let (id, i, detail, ctx) = self.next_job();
            let Some(t) = detail.ready().and_then(|d| d.tracks.get(i).cloned()) else {
                self.finish(&id, i);
                continue;
            };
            if t.source != "spotify" || t.unavailable || !t.resolving {
                self.finish(&id, i);
                continue;
            }
            let path = format!(
                "/youtube/match?id={}&title={}&artist={}&duration={}&album={}",
                backend::enc(&t.id),
                backend::enc(&t.title),
                backend::enc(&t.artists.join(", ")),
                t.duration_sec.map_or(String::new(), |d| d.to_string()),
                backend::enc(t.album.as_deref().unwrap_or("")),
            );
            let answer = backend::get::<Match>(&path);
            let retry = match &answer {
                Ok(m) => m.retry,
                Err(_) => true,
            };
            if let State::Ready(d) = &mut *detail.lock() {
                if let Some(t) = Arc::make_mut(d).tracks.get_mut(i) {
                    match answer {
                        Ok(m) => match m.track {
                            Some(yt) => *t = yt,
                            None => {
                                t.resolving = false;
                                t.unavailable = true;
                            }
                        },
                        Err(_) => {}
                    }
                }
            }
            if !retry {
                self.done.fetch_add(1, Ordering::Relaxed);
            }
            ctx.request_repaint();
            if retry {
                std::thread::sleep(Duration::from_secs(3));
            }
            self.finish(&id, i);
        }
    }

    fn next_job(&self) -> (String, usize, Arc<Remote<Detail>>, Context) {
        let mut windows = self.windows.lock().unwrap();
        loop {
            for (id, w) in windows.iter_mut() {
                if w.seen.elapsed() > Duration::from_secs(3) {
                    w.wanted.clear();
                }
                while let Some(i) = w.wanted.pop_first() {
                    if w.active.insert(i) {
                        return (id.clone(), i, w.detail.clone(), w.ctx.clone());
                    }
                }
            }
            windows = self.wake.wait(windows).unwrap();
        }
    }

    fn finish(&self, id: &str, i: usize) {
        if let Some(w) = self.windows.lock().unwrap().get_mut(id) {
            w.active.remove(&i);
        }
    }

    fn clear(&self) {
        self.windows.lock().unwrap().clear();
    }

    /// Lookups still queued or running.
    fn pending(&self) -> usize {
        self.windows.lock().unwrap().values().map(|w| w.wanted.len() + w.active.len()).sum()
    }
}

/// Query completions and the songs, albums and artists YouTube Music suggests for a half-typed search.
#[derive(Deserialize, Clone, Default)]
#[serde(default)]
pub struct Suggest {
    pub queries: Vec<String>,
    pub items: Vec<Item>,
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

impl Detail {
    /// What playing this page plays: its track list, or for artists the songs on their shelves.
    fn clean(mut self) -> Self {
        self.tracks.retain(|t| !hidden(t.explicit, t.video));
        clean_sections(&mut self.sections);
        self
    }

    pub fn songs(&self) -> Vec<Track> {
        if !self.tracks.is_empty() {
            return self.tracks.clone();
        }
        self.sections.iter().flat_map(|s| &s.items).filter(|i| i.r#type == "song").map(|i| i.track()).collect()
    }
}

/// YouTube Music history plus aggregates, computed once on load.
#[derive(Default)]
pub struct Stats {
    pub sections: Vec<Section>,
    pub plays: usize,
    pub songs: usize,
    pub artists: usize,
    pub secs: u32,
    /// Name, browse id (known when they were a song's first artist) and plays.
    pub top_artists: Vec<(String, Option<String>, usize)>,
    pub top_songs: Vec<(Item, usize)>,
}

impl Stats {
    pub fn new(sections: Vec<Section>) -> Self {
        let mut songs: HashMap<&str, (&Item, usize)> = HashMap::new();
        let mut artists: HashMap<&str, (Option<&str>, usize)> = HashMap::new();
        let (mut plays, mut secs) = (0, 0);
        for it in sections.iter().flat_map(|s| &s.items).filter(|i| i.r#type == "song") {
            plays += 1;
            secs += it.duration_sec.unwrap_or(0);
            songs.entry(&it.id).or_insert((it, 0)).1 += 1;
            for (i, a) in it.subtitle.as_deref().unwrap_or("").split(", ").filter(|a| !a.is_empty()).enumerate() {
                let e = artists.entry(a).or_default();
                e.1 += 1;
                if i == 0 && e.0.is_none() {
                    e.0 = it.artist_id.as_deref();
                }
            }
        }
        let mut top_artists: Vec<_> = artists.iter().map(|(k, (id, n))| (k.to_string(), id.map(String::from), *n)).collect();
        top_artists.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
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
    pub settings: Remote<crate::settings::Daemon>,
    pub home: Remote<Vec<Section>>,
    pub history: Remote<Stats>,
    /// Number of Spotify playlists (its listening history is not exposed).
    pub spotify: Remote<Vec<Item>>,
    /// Saved playlists, albums and artists for the sidebar, tagged with their filter index (1 playlist, 2 album, 3 artist).
    pub library: Remote<Vec<(usize, Item)>>,
    /// Bumped whenever `library` is edited in place, so the sidebar rebuilds its copy.
    pub lib_rev: AtomicUsize,
    /// Song search results by query.
    pub searches: Mutex<HashMap<String, Arc<Remote<Vec<Track>>>>>,
    /// Album and artist search results by `type/query`.
    pub item_searches: Mutex<HashMap<String, Arc<Remote<Vec<Item>>>>>,
    /// Error from the last library save, shown on the album page.
    pub details: Mutex<HashMap<String, Arc<Remote<Detail>>>>,
    /// Search suggestions by query.
    pub suggests: Mutex<HashMap<String, Arc<Remote<Suggest>>>>,
    matcher: Arc<SpotifyMatcher>,
    exporting: Arc<AtomicBool>,
    /// Songs looked up by id, to learn which album a feed item sits on.
    pub song_infos: Mutex<HashMap<String, Arc<Remote<Track>>>>,
    /// Set while a sign-in/out is in flight (the login window blocks), with the last error.
    pub busy: Arc<Mutex<(Option<&'static str>, Option<String>)>>,
}

impl Data {
    pub fn new() -> Self {
        Data {
            status: Remote::new(),
            settings: Remote::new(),
            home: Remote::new(),
            history: Remote::new(),
            spotify: Remote::new(),
            library: Remote::new(),
            lib_rev: AtomicUsize::new(0),
            searches: Default::default(),
            item_searches: Default::default(),
            details: Default::default(),
            suggests: Default::default(),
            matcher: SpotifyMatcher::start(),
            exporting: Default::default(),
            song_infos: Default::default(),
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
            r.ensure(ctx, move || {
                let tracks: Vec<Track> = backend::get(&path)?;
                Ok(Detail { item, tracks, ..Default::default() })
            });
        } else {
            let path = format!("/youtube/{kind}/{id}");
            r.ensure(ctx, move || backend::get::<Detail>(&path).map(Detail::clean));
        }
        r
    }

    /// Queues the visible Spotify rows (± padding, filtered in page.rs) for matching.
    pub fn want_spotify(&self, ctx: &Context, id: &str, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let detail = self.details.lock().unwrap().get(&format!("spotify/{id}")).cloned();
        if let Some(detail) = detail {
            self.matcher.request(ctx, id, &detail, range);
        }
    }

    /// Creates a YouTube Music playlist from the rows that already matched (unmatched rows are reported, not waited on).
    pub fn export_spotify(self: &Arc<Self>, ctx: &Context, title: &str, tracks: &[Track]) {
        if self.exporting.swap(true, Ordering::SeqCst) {
            crate::toast::info("An export is already running");
            return;
        }
        let video_ids: Vec<String> = tracks.iter().filter(|t| t.source == "youtube" && !t.id.is_empty()).map(|t| t.id.clone()).collect();
        if video_ids.is_empty() {
            self.exporting.store(false, Ordering::SeqCst);
            crate::toast::error("No matched tracks to export yet");
            return;
        }
        let title = if title.trim().is_empty() { "Spotify playlist" } else { title.trim() }.to_string();
        let total = tracks.len();
        crate::toast::info(format!("Exporting {} tracks to YouTube Music…", video_ids.len()));
        let exporting = self.exporting.clone();
        let (me, ctx) = (self.clone(), ctx.clone());
        std::thread::spawn(move || {
            let req = ExportReq { title: &title, video_ids: &video_ids };
            match backend::post_json::<_, ExportRes>("/youtube/playlists", &req) {
                Ok(r) => {
                    let skipped = total.saturating_sub(r.added);
                    let msg = if skipped > 0 {
                        format!("Created “{title}” with {} of {total} tracks ({skipped} skipped)", r.added)
                    } else {
                        format!("Created “{title}” with {} tracks", r.added)
                    };
                    crate::toast::info(msg);
                    let item = Item { r#type: "playlist".into(), id: r.id, title: title.clone(), editable: true, ..Default::default() };
                    me.set_saved(1, item, true);
                    me.library.reset();
                }
                Err(e) => crate::toast::error(format!("Couldn't export the playlist: {e}")),
            }
            exporting.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// Deletes a playlist the user owns. It leaves the sidebar at once; a refusal puts it back, success refetches the library.
    pub fn delete_playlist(self: &Arc<Self>, ctx: &Context, id: &str) {
        let item = self.library_item(id).unwrap_or_else(|| self.known_item("playlists", id, "playlist"));
        self.set_saved(1, item.clone(), false);
        let (me, ctx) = (self.clone(), ctx.clone());
        let path = format!("/youtube/playlists/{}", backend::enc(id));
        std::thread::spawn(move || {
            match backend::delete(&path) {
                Ok(()) => {
                    crate::toast::info("Playlist deleted");
                    me.details.lock().unwrap().remove(&format!("playlists/{}", item.id));
                    me.library.reset();
                }
                Err(e) => {
                    me.set_saved(1, item, true);
                    crate::toast::error(format!("Couldn't delete the playlist: {e}"));
                }
            }
            ctx.request_repaint();
        });
    }

    pub fn library_item(&self, id: &str) -> Option<Item> {
        match &*self.library.lock() {
            State::Ready(v) => v.iter().find(|(k, i)| *k == 1 && i.id == id).map(|(_, i)| i.clone()),
            _ => None,
        }
    }

    pub fn song_info(&self, ctx: &Context, id: &str) -> Arc<Remote<Track>> {
        let r = self.song_infos.lock().unwrap().entry(id.to_string()).or_insert_with(|| Arc::new(Remote::new())).clone();
        let path = format!("/youtube/song/{}", backend::enc(id));
        r.ensure(ctx, move || backend::get(&path));
        r
    }

    pub fn search(&self, ctx: &Context, q: &str) -> Arc<Remote<Vec<Track>>> {
        let r = self.searches.lock().unwrap().entry(q.to_string()).or_insert_with(|| Arc::new(Remote::new())).clone();
        let path = format!("/youtube/search?q={}", backend::enc(q));
        r.ensure(ctx, move || backend::get::<Vec<Track>>(&path).map(|mut v| {
            v.retain(|t| !hidden(t.explicit, t.video));
            v
        }));
        r
    }

    pub fn suggest(&self, ctx: &Context, q: &str) -> Arc<Remote<Suggest>> {
        let r = self.suggests.lock().unwrap().entry(q.to_string()).or_insert_with(|| Arc::new(Remote::new())).clone();
        let path = format!("/youtube/suggest?q={}", backend::enc(q));
        r.ensure(ctx, move || backend::get::<Suggest>(&path).map(|mut s| {
            s.items.retain(|i| !hidden(i.explicit, i.video));
            s
        }));
        r
    }

    /// Spotify rows still being matched to YouTube Music, and how many finished since `import_collect`.
    pub fn import_progress(&self) -> (usize, usize) {
        (self.matcher.done.load(Ordering::Relaxed), self.matcher.pending())
    }

    /// Takes the finished-lookup count, starting the next run from zero.
    pub fn import_collect(&self) -> usize {
        self.matcher.done.swap(0, Ordering::Relaxed)
    }

    /// Albums or artists (`kind` is the search filter: album | artist) matching `q`.
    pub fn search_items(&self, ctx: &Context, q: &str, kind: &'static str) -> Arc<Remote<Vec<Item>>> {
        let r = self.item_searches.lock().unwrap().entry(format!("{kind}/{q}")).or_insert_with(|| Arc::new(Remote::new())).clone();
        let path = format!("/youtube/search/items?q={}&type={kind}", backend::enc(q));
        r.ensure(ctx, move || backend::get::<Vec<Item>>(&path).map(|mut v| {
            v.retain(|i| !hidden(i.explicit, i.video));
            v
        }));
        r
    }

    /// Drops what was fetched under the old content filters so it is fetched again (Spotify imports keep their match progress).
    pub fn refilter(&self) {
        self.home.reset();
        self.details.lock().unwrap().retain(|k, _| k.starts_with("spotify/"));
        self.searches.lock().unwrap().clear();
        self.item_searches.lock().unwrap().clear();
        self.suggests.lock().unwrap().clear();
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
        self.matcher.clear();
        self.song_infos.lock().unwrap().clear();
    }

    /// Adds or removes a sidebar entry (`n`: 1 playlist, 2 album, 3 artist) right away, without waiting on YouTube's eventually-consistent library.
    fn set_saved(&self, n: usize, item: Item, on: bool) {
        if let State::Ready(v) = &mut *self.library.lock() {
            let v = Arc::make_mut(v);
            v.retain(|(k, i)| !(*k == n && i.id == item.id));
            if on {
                v.insert(0, (n, item));
            }
            self.lib_rev.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The loaded page item for `kind/id`, else a bare one.
    fn known_item(&self, kind: &str, id: &str, r#type: &str) -> Item {
        let page = self.details.lock().unwrap().get(&format!("{kind}/{id}")).cloned();
        if let Some(p) = page {
            if let State::Ready(d) = &*p.lock() {
                return Item { r#type: r#type.into(), id: id.into(), ..d.item.clone() };
            }
        }
        Item { r#type: r#type.into(), id: id.into(), ..Default::default() }
    }

    /// Adds or removes an album (`kind` albums) or playlist from the YouTube Music library. The sidebar and toast update at once; a refusal puts it back.
    pub fn save(self: &Arc<Self>, ctx: &Context, kind: &'static str, id: &str, on: bool) {
        if !on && kind == "playlists" && self.library_item(id).is_some_and(|i| i.editable) {
            return self.delete_playlist(ctx, id);
        }
        let (n, ty) = if kind == "albums" { (2, "album") } else { (1, "playlist") };
        let item = self.known_item(kind, id, ty);
        self.set_saved(n, item.clone(), on);
        crate::toast::info(if on { "Saved to your library" } else { "Removed from your library" });
        let (me, ctx) = (self.clone(), ctx.clone());
        let path = format!("/youtube/save/{kind}/{id}?on={on}");
        std::thread::spawn(move || {
            if let Err(e) = backend::post(&path) {
                me.set_saved(n, item, !on);
                crate::toast::error(format!("Couldn't update your library: {e}"));
            }
            ctx.request_repaint();
        });
    }

    /// Whether `id` is in the loaded Liked songs page (false until it loads).
    pub fn is_liked(&self, id: &str) -> bool {
        let page = self.details.lock().unwrap().get("playlists/LM").cloned();
        page.is_some_and(|p| matches!(&*p.lock(), State::Ready(d) if d.tracks.iter().any(|t| t.id == id)))
    }

    /// Likes or unlikes a song from a list row: the star and Liked songs page change at once; a refusal puts it back.
    pub fn like(self: &Arc<Self>, ctx: &Context, track: Track, on: bool) {
        self.mirror_like(track.clone(), on);
        crate::toast::info(if on { "Added to liked songs" } else { "Removed from liked songs" });
        let (me, ctx) = (self.clone(), ctx.clone());
        let path = format!("/youtube/like/{}?on={on}", backend::enc(&track.id));
        std::thread::spawn(move || {
            if let Err(e) = backend::post(&path) {
                me.mirror_like(track, !on);
                crate::toast::error(format!("Couldn't update your likes: {e}"));
            }
            ctx.request_repaint();
        });
    }

    /// Mirrors a like into the cached Liked songs page (`LM`) so it shows without a refetch.
    pub fn mirror_like(&self, track: Track, on: bool) {
        for key in ["playlists/LM", "playlists/VLLM"] {
            let page = self.details.lock().unwrap().get(key).cloned();
            if let Some(p) = page {
                if let State::Ready(d) = &mut *p.lock() {
                    let d = Arc::make_mut(d);
                    d.tracks.retain(|t| t.id != track.id);
                    if on {
                        d.tracks.insert(0, track.clone());
                    }
                }
            }
        }
    }

    /// Subscribes to (or leaves) an artist's channel. The button, sidebar and toast update at once; a refusal puts it back.
    pub fn subscribe(self: &Arc<Self>, ctx: &Context, artist_id: &str, channel_id: &str, on: bool) {
        let (me, ctx) = (self.clone(), ctx.clone());
        let (path, key) = (format!("/youtube/subscribe/{channel_id}?on={on}"), format!("artists/{artist_id}"));
        let item = self.known_item("artists", artist_id, "artist");
        let set = move |me: &Data, v: bool| {
            let page = me.details.lock().unwrap().get(&key).cloned();
            if let Some(d) = page {
                if let State::Ready(d) = &mut *d.lock() {
                    Arc::make_mut(d).subscribed = v;
                }
            }
            me.set_saved(3, item.clone(), v);
        };
        set(&me, on);
        crate::toast::info(if on { "Subscribed" } else { "Unsubscribed" });
        std::thread::spawn(move || {
            if let Err(e) = backend::post(&path) {
                set(&me, !on);
                crate::toast::error(format!("Couldn't update the subscription: {e}"));
            }
            ctx.request_repaint();
        });
    }

    /// Runs the blocking sign-in/out on a worker, then drops every cached result so pages refetch.
    pub fn account(self: &Arc<Self>, ctx: &Context, svc: &'static str, sign_in: bool) {
        let (me, ctx) = (self.clone(), ctx.clone());
        *me.busy.lock().unwrap() = (Some(svc), None);
        std::thread::spawn(move || {
            let r = if sign_in { backend::sign_in(svc) } else { backend::sign_out(svc) };
            let err = r.as_ref().err().cloned();
            match r {
                Ok(()) => crate::toast::info(format!("{} {}", if sign_in { "Signed in to" } else { "Signed out of" }, if svc == "youtube" { "YouTube Music" } else { "Spotify" })),
                Err(e) => crate::toast::error(e),
            }
            *me.busy.lock().unwrap() = (None, err);
            me.reset_content();
            ctx.request_repaint();
        });
    }
}
