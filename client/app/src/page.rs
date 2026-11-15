//! Centre panel: album page (header wash, actions, virtualized track list).

use crate::theme::Theme;
use gpui::prelude::*;
use gpui::*;
use std::ops::Range;

const ROW_H: f32 = 44.;

struct Track {
    title: &'static str,
    artist: &'static str,
    dur: &'static str,
}

pub struct AlbumPage {
    tracks: Vec<Track>,
    playing: usize,
}

impl AlbumPage {
    pub fn new() -> Self {
        let t = |title, artist, dur| Track { title, artist, dur };
        AlbumPage {
            playing: 1,
            tracks: vec![
                t("Last Stop Is Mine", "Hoshi Yume", "3:42"),
                t("Window Seat, Rain", "Hoshi Yume", "4:08"),
                t("Transfer at Midnight", "Hoshi Yume, Aoi Tsuru", "3:15"),
                t("Fare Card Blue", "Hoshi Yume", "2:57"),
                t("Depot", "Hoshi Yume", "5:21"),
                t("Everyone Gets Off Here, Eventually", "Hoshi Yume", "3:39"),
                t("Terminus", "Hoshi Yume", "4:44"),
                t("Paper Lantern", "Hoshi Yume", "3:20"),
            ],
        }
    }

    fn rows(&mut self, range: Range<usize>, t: &Theme) -> Vec<Stateful<Div>> {
        range.map(|ix| {
            let tr = &self.tracks[ix];
            let on = ix == self.playing;
            row(t, 
                if on { eq(t) } else { div().child((ix + 1).to_string()) },
                div().child(tr.title).when(on, |d| d.text_color(t.ax).font_weight(FontWeight::MEDIUM)),
                div().text_color(t.tm).child(tr.artist),
                div().text_color(t.tm).text_right().child(tr.dur),
            ).h(px(ROW_H)).id(("row", ix)).hover(|s| s.bg(t.s2)).active(|s| s.bg(t.s3))
        }).collect()
    }
}

/// Columns: # | title (1.6) | artist (1) | time. Shared by the header and rows so they line up.
fn row(t: &Theme, n: Div, title: Div, artist: Div, dur: Div) -> Div {
    let mut title = title.truncate().min_w_0();
    title.style().flex_grow = Some(1.6);
    title.style().flex_basis = Some(px(0.).into());
    let mut artist = artist.truncate().min_w_0();
    artist.style().flex_grow = Some(1.);
    artist.style().flex_basis = Some(px(0.).into());
    div().w_full().flex().items_center().gap(px(8.)).px(px(12.)).rounded(t.ctl_r)
        .child(n.w(px(36.)).flex_none().text_color(t.tm))
        .child(title).child(artist).child(dur.w(px(52.)).flex_none())
}

fn eq(t: &Theme) -> Div {
    // ponytail: static bars, animate with an Animation per bar once playback state exists
    div().flex().gap(px(2.)).items_end().h(px(12.)).children([0.9, 0.5, 0.75].map(|h| div().w(px(2.)).h(relative(h)).rounded(px(1.)).bg(t.ax)))
}

impl Render for AlbumPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();

        let wash = div().flex().gap(px(22.)).items_end().px(px(22.)).pt(px(24.)).pb(px(20.)).rounded(px(18.)).bg(t.wash())
            .child(div().size(px(148.)).flex_none().rounded(t.art_r).bg(t.art(0.)).shadow_md())
            .child(div().min_w_0().flex().flex_col().gap(px(6.))
                .child(div().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.tm).child("ALBUM · 2024"))
                .child(div().text_size(px(30.)).line_height(relative(1.05)).font_weight(FontWeight(650.)).child("Night Bus Home"))
                .child(div().text_color(t.tm).child("Hoshi Yume · 11 songs · 41 min")));

        let ghost = |id: &'static str, label: &'static str| {
            div().id(id).px(px(12.)).py(px(8.)).rounded(t.ctl_r).text_color(t.tm).cursor_pointer()
                .hover(|s| s.bg(t.s2).text_color(t.tx)).active(|s| s.bg(t.s3)).child(label)
        };
        let acts = div().flex().gap(px(8.)).items_center().px(px(22.)).pt(px(14.)).pb(px(8.))
            .child(div().id("play").flex().items_center().gap(px(8.)).pl(px(14.)).pr(px(18.)).py(px(8.)).rounded_full().bg(t.ax).text_color(t.on_ax).font_weight(FontWeight::SEMIBOLD).cursor_pointer()
                .active(|s| s.opacity(0.85))
                .child(icon!("play", 16., t.on_ax)).child("Play"))
            .child(ghost("shuffle", "Shuffle")).child(ghost("save", "Save")).child(ghost("download", "Download"));

        let head = row(&t, div().child("#"), div().child("TITLE"), div().child("ARTIST"), div().text_right().child("TIME"))
            .h(px(28.)).text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.tf);

        let n = self.tracks.len();
        div().size_full().flex().flex_col().overflow_hidden().relative()
            .child(wash).child(acts)
            .child(div().flex_1().min_h_0().flex().flex_col().px(px(6.)).child(head).child(
                uniform_list("tracks", n, cx.processor(|this, range, _, cx| { let t = *cx.global::<Theme>(); this.rows(range, &t) })).flex_1(),
            ))
            .child(div().absolute().left_0().right_0().bottom_0().h(px(56.)).bg(Theme::fade(t.s1)))
    }
}
