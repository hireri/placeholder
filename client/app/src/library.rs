//! Left sidebar: page nav plus a virtualized list of saved items. Stays laid out at its
//! own width, so collapsing only clips text and leaves icons and covers where they were.

use crate::theme::Theme;
use crate::ui::{art, icon_button};
use gpui::prelude::*;
use gpui::*;
use std::ops::Range;

pub struct Toggled;

struct Item {
    title: SharedString,
    sub: SharedString,
    hue: f32,
    round: bool,
}

pub struct Library {
    pub open: bool,
    page: usize,
    filter: usize,
    current: usize,
    items: Vec<Item>,
}

impl EventEmitter<Toggled> for Library {}

const NAV: [(&str, &str); 5] = [("Home", "icons/home.svg"), ("Library", "icons/lib.svg"), ("Downloads", "icons/imp.svg"), ("Local Music", "icons/dir.svg"), ("History", "icons/hist.svg")];
const FILTERS: [&str; 4] = ["All", "Playlists", "Albums", "Artists"];

impl Library {
    pub fn new() -> Self {
        let it = |t: &str, s: &str, hue: f32, round: bool| Item { title: t.to_string().into(), sub: s.to_string().into(), hue, round };
        let mut items = vec![
            it("Liked songs", "Playlist · 812 songs", -20., false),
            it("Night Bus Home", "Album · Hoshi Yume", 50., false),
            it("Hoshi Yume", "Artist", 120., true),
            it("Train windows", "Playlist · 42 songs", 160., false),
            it("Night shift", "Playlist · from Spotify", 210., false),
            it("Aoi Tsuru", "Artist", -70., true),
            it("Paper Lantern EP", "Album · Hoshi Yume", 80., false),
        ];
        items.extend((0..300).map(|i| it(&format!("Playlist {i}"), &format!("Playlist · {} songs", 10 + i % 90), (i * 37 % 360) as f32, false)));
        Library { open: true, page: 1, filter: 0, current: 1, items }
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.emit(Toggled);
        cx.notify();
    }

    fn rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let t = *cx.global::<Theme>();
        range.map(|ix| {
            let it = &self.items[ix];
            div().id(("li", ix)).w_full().h(px(50.)).flex_none().flex().items_center().gap(px(10.)).px(px(6.)).rounded(px(10.)).overflow_hidden().cursor_pointer()
                .when(ix == self.current, |d| d.bg(t.axc))
                .hover(|s| s.bg(t.s2)).active(|s| s.bg(t.s3))
                .on_click(cx.listener(move |this, _, _, cx| { this.current = ix; cx.notify(); }))
                .child(art(&t, 40., it.hue, it.round))
                .child(div().w(px(186.)).flex_none().line_height(relative(1.3))
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(it.title.clone()))
                    .child(div().truncate().text_size(px(12.)).text_color(t.tm).child(it.sub.clone())))
        }).collect()
    }
}

impl Render for Library {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let open = self.open;

        let nav = div().flex().flex_col().gap(px(1.)).pb(px(6.)).flex_none().children(NAV.iter().enumerate().map(|(i, (label, icon))| {
            let on = i == self.page;
            div().id(("nav", i)).w_full().h(px(38.)).flex().items_center().gap(px(21.)).pl(px(17.)).rounded(px(10.)).overflow_hidden().whitespace_nowrap().cursor_pointer()
                .text_color(if on { t.tx } else { t.tm })
                .when(on, |d| d.bg(t.axc).font_weight(FontWeight::MEDIUM))
                .hover(|s| s.bg(t.s2).text_color(t.tx)).active(|s| s.bg(t.s3))
                .on_click(cx.listener(move |this, _, _, cx| { this.page = i; cx.notify(); }))
                .child(svg().path(*icon).w(px(18.)).h(px(18.)).flex_none().text_color(if on { t.tx } else { t.tm }))
                .child(*label)
        }));

        let header = div().h(px(32.)).flex_none().flex().items_center().gap(px(14.)).pl(px(10.)).overflow_hidden()
            .child(icon_button("lib-toggle", &t, 32., icon!("col", 16., t.tm)).on_click(cx.listener(|this, _, _, cx| this.toggle(cx))))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).whitespace_nowrap().child("Your library"));

        // Collapsed, the search is just an icon that expands the sidebar (focus-on-expand comes with a real input).
        let search = div().id("lib-search").h(px(32.)).flex_none().flex().items_center().gap(px(21.)).pl(px(17.)).rounded(t.ctl_r).overflow_hidden().text_color(t.tm).cursor_pointer()
            .when(open, |d| d.bg(t.s2))
            .when(!open, |d| d.hover(|s| s.bg(t.s2)).on_click(cx.listener(|this, _, _, cx| this.toggle(cx))))
            .child(icon!("search", 18., t.tm))
            .child(div().w(px(182.)).flex_none().text_size(px(12.5)).whitespace_nowrap().child("Search your library"));

        let filters = div().flex().gap(px(4.)).px(px(6.)).py(px(2.)).flex_none().overflow_hidden().children(FILTERS.iter().enumerate().map(|(i, f)| {
            let on = i == self.filter;
            div().id(("filter", i)).h(px(26.)).px(px(10.)).flex().items_center().rounded_full().text_size(px(12.)).whitespace_nowrap().cursor_pointer()
                .bg(if on { t.tx } else { t.s2 }).text_color(if on { t.s0 } else { t.tm })
                .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                .when(!on, |d| d.hover(|s| s.text_color(t.tx)))
                .on_click(cx.listener(move |this, _, _, cx| { this.filter = i; cx.notify(); }))
                .child(*f)
        }));

        let list = div().relative().flex_1().min_h_0().child(
            uniform_list("lib-items", self.items.len(), cx.processor(|this, range, _, cx| this.rows(range, cx))).size_full(),
        ).child(div().absolute().left_0().right_0().bottom_0().h(px(40.)).bg(Theme::fade(t.s0)));

        div().size_full().flex().flex_col().gap_2().px(px(8.)).pt(px(12.)).overflow_hidden()
            .child(nav).child(header).child(search).when(open, |d| d.child(filters)).child(list)
    }
}
