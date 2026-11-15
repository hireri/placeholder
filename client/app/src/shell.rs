//! Window shell: titlebar, collapsible library (left), page (centre), collapsible player (right).
//!
//! Each region is its own entity rendered through `AnyView::cached`, so a change in one
//! (a playing-row tick, a library click) never re-renders the others. Collapsing animates only
//! the outer slot width; the player keeps its fixed width inside a clipped slot, so it is a cache hit every frame.

use crate::library::{Library, Toggled};
use crate::page::AlbumPage;
use crate::player::{Hide, Player, PlayerView, WIDTH as PLAYER_W};
use crate::theme::Theme;
use crate::ui::{art, bar, icon_button};
use gpui::prelude::*;
use gpui::*;
use std::time::Duration;

const LIB_OPEN: f32 = 264.;
const LIB_SHUT: f32 = 68.;

pub struct Shell {
    lib: Entity<Library>,
    page: Entity<AlbumPage>,
    player: Entity<Player>,
    player_open: bool,
    /// Bumped on each toggle so the width animation restarts; 0 means "no animation yet" (initial layout).
    lib_epoch: u32,
    player_epoch: u32,
    _subs: [Subscription; 2],
}

/// A slot whose width animates between `shut` and `full`, clipping its contents.
fn slot(el: Div, id: &'static str, epoch: u32, open: bool, full: f32, shut: f32) -> AnyElement {
    let (from, to) = if open { (shut, full) } else { (full, shut) };
    let el = el.h_full().flex_none().overflow_hidden();
    if epoch == 0 {
        return el.w(px(to)).into_any_element();
    }
    el.w(px(from))
        .with_animation((id, epoch), Animation::new(Duration::from_millis(320)).with_easing(ease_out_quint()), move |el, d| el.w(px(from + (to - from) * d)))
        .into_any_element()
}

fn cached<V: Render>(e: &Entity<V>) -> AnyView {
    AnyView::from(e.clone()).cached(StyleRefinement::default().size_full())
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let lib = cx.new(|_| Library::new());
        let player = cx.new(|_| Player::new());
        let page = cx.new(|_| AlbumPage::new());
        let subs = [
            cx.subscribe(&lib, |this, _, _: &Toggled, cx| { this.lib_epoch += 1; cx.notify(); }),
            cx.subscribe(&player, |this, _, _: &Hide, cx| this.set_player(None, cx)),
        ];
        Shell { lib, page, player, player_open: true, lib_epoch: 0, player_epoch: 0, _subs: subs }
    }

    /// `Some(view)` opens the player on that view; `None` hides it.
    fn set_player(&mut self, open_on: Option<PlayerView>, cx: &mut Context<Self>) {
        if let Some(v) = open_on {
            self.player.update(cx, |p, cx| p.set_view(v, cx));
        }
        if self.player_open != open_on.is_some() {
            self.player_open = open_on.is_some();
            self.player_epoch += 1;
        }
        cx.notify();
    }

    fn titlebar(&self, t: &Theme, lib_open: bool) -> Div {
        let brand = slot(
            div().flex().items_center().gap(px(12.)).pl(px(26.)).whitespace_nowrap().font_weight(FontWeight::SEMIBOLD).text_size(px(14.))
                .child(div().size(px(16.)).flex_none().rounded(px(5.)).bg(t.ax)).child("Metrolist"),
            "brand", self.lib_epoch, lib_open, LIB_OPEN, LIB_SHUT,
        );
        // Search is a visual stand-in until there is a text input (gpui ships none).
        let mid = div().flex_1().min_w_0().flex().justify_center().child(
            div().w_full().max_w(px(560.)).flex().items_center().gap(px(6.))
                .child(icon_button("back", t, 32., icon!("back", 16., t.tm)))
                .child(icon_button("fwd", t, 32., icon!("fwd", 16., t.tm)))
                .child(div().flex_1().h(px(34.)).flex().items_center().gap_2().px(px(14.)).rounded_full().bg(t.s2).text_color(t.tm)
                    .child(icon!("search", 16., t.tm))
                    .child(div().flex_1().child("Search songs, albums, artists"))
                    .child(div().text_size(px(10.5)).text_color(t.tf).child("Ctrl K"))),
        );
        let right = div().flex().items_center().gap(px(6.))
            .child(div().h(px(28.)).flex().items_center().gap_2().px(px(10.)).rounded_full().bg(t.s1).text_size(px(12.)).text_color(t.tm)
                .child(icon!("imp", 16., t.tm)).child("Importing 412 / 640").child(div().w(px(44.)).child(bar(t.s3, t.ax, 3., 0.64))))
            .child(icon_button("bell", t, 32., icon!("bell", 16., t.tm)))
            .child(div().id("me").flex().items_center().gap_2().py(px(3.)).pl(px(3.)).pr(px(10.)).rounded_full().cursor_pointer().hover(|s| s.bg(t.s2))
                .child(div().size(px(26.)).flex().items_center().justify_center().rounded_full().bg(t.axc).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child("HY"))
                .child("Hireri"));
        div().h(px(52.)).flex_none().flex().items_center().pr(px(12.)).child(brand).child(mid).child(right)
    }

    /// Floating pill shown while the player is collapsed; centred on the window.
    fn mini(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let open = |v: PlayerView| cx.listener(move |this, _, _, cx| this.set_player(Some(v), cx));
        let pill = div().relative().flex().items_center().gap(px(10.)).pl(px(8.)).pr(px(14.)).pt(px(8.)).pb(px(12.)).rounded_full().bg(t.s2).shadow_lg()
            .child(div().id("mini-who").flex().items_center().gap(px(10.)).pr(px(8.)).rounded_full().cursor_pointer().on_click(open(PlayerView::Art))
                .child(art(t, 40., 50., true))
                .child(div().w(px(132.)).line_height(relative(1.3))
                    .child(div().truncate().font_weight(FontWeight::SEMIBOLD).child("Window Seat, Rain"))
                    .child(div().truncate().text_size(px(12.)).text_color(t.tm).child("Hoshi Yume"))))
            .child(div().flex().items_center().gap(px(2.))
                .child(icon_button("mini-prev", t, 32., icon!("rw", 16., t.tx)))
                .child(div().id("mini-pp").size(px(36.)).flex().items_center().justify_center().rounded_full().bg(t.tx).cursor_pointer().active(|s| s.opacity(0.85)).child(icon!("play", 16., t.s0)))
                .child(icon_button("mini-next", t, 32., icon!("ff", 16., t.tx))))
            .child(div().w(px(1.)).h(px(22.)).bg(t.s3))
            .child(icon_button("mini-lyr", t, 32., icon!("lyr", 16., t.tm)).on_click(open(PlayerView::Lyrics)))
            .child(icon_button("mini-que", t, 32., icon!("que", 16., t.tm)).on_click(open(PlayerView::Queue)))
            .child(div().absolute().left(px(24.)).right(px(24.)).bottom(px(5.)).child(bar(t.s3, t.tx, 2., 0.38)));
        div().absolute().left_0().right_0().bottom(px(14.)).flex().justify_center().child(pill)
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let lib_open = self.lib.read(cx).open;

        let body = div().flex_1().min_h_0().flex()
            .child(slot(div().child(cached(&self.lib)), "lib", self.lib_epoch, lib_open, LIB_OPEN, LIB_SHUT))
            .child(div().flex_1().min_w_0().mb(px(10.)).rounded(px(18.)).bg(t.s1).shadow_lg().overflow_hidden().child(cached(&self.page)))
            .child(slot(div().child(div().w(px(PLAYER_W)).h_full().child(cached(&self.player))), "player", self.player_epoch, self.player_open, PLAYER_W, 0.));

        div().size_full().relative().flex().flex_col().bg(t.s0).text_color(t.tx).text_size(px(13.)).line_height(relative(1.45))
            .child(self.titlebar(&t, lib_open)).child(body)
            .when(!self.player_open, |d| d.child(self.mini(&t, cx)))
    }
}
