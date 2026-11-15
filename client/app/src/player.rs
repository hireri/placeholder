//! Right sidebar: cover/lyrics/queue switcher with transport. Fixed 308px wide; the shell
//! slides the column in and out around it, so this view never re-lays out while animating.

use crate::theme::Theme;
use crate::ui::{art, bar, icon_button};
use gpui::prelude::*;
use gpui::*;

pub const WIDTH: f32 = 308.;

#[derive(Clone, Copy, PartialEq)]
pub enum PlayerView {
    Art,
    Lyrics,
    Queue,
}

pub struct Hide;

pub struct Player {
    view: PlayerView,
}

impl EventEmitter<Hide> for Player {}

const LYRICS: [&str; 6] = [
    "Fogged glass, the city blurs",
    "I trace your name along the pane",
    "Every light we pass is yours",
    "Every light I count again",
    "Driver, take the long way round",
    "Past the depot, past the sea",
];
const QUEUE: [(&str, &str); 4] = [("Transfer at Midnight", "Hoshi Yume, Aoi Tsuru"), ("Fare Card Blue", "Hoshi Yume"), ("Depot", "Hoshi Yume"), ("Everyone Gets Off Here, Eventually", "Hoshi Yume")];

impl Player {
    pub fn new() -> Self {
        Player { view: PlayerView::Art }
    }

    pub fn set_view(&mut self, view: PlayerView, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    fn lyrics(&self, t: &Theme) -> Div {
        div().flex_1().min_h_0().flex().flex_col().gap(px(14.)).py(px(10.)).px(px(2.)).overflow_hidden()
            .text_size(px(21.)).line_height(relative(1.22)).font_weight(FontWeight(650.))
            .children(LYRICS.iter().enumerate().map(|(i, l)| div().text_color(if i == 2 { t.tx } else { t.tf }).child(*l)))
    }

    fn queue(&self, t: &Theme) -> Div {
        let label = |s: &'static str| div().pt(px(8.)).pb(px(4.)).text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.tf).child(s);
        let item = |ix: usize, title: &'static str, artist: &'static str, on: bool| {
            div().h(px(52.)).flex_none().flex().items_center().gap(px(10.)).px(px(4.)).rounded(t.ctl_r).hover(|s| s.bg(t.s2))
                .child(div().w(px(14.)).flex_none().when(!on, |d| d.child(icon!("grip", 14., t.tf))))
                .child(art(t, 38., 50., false))
                .child(div().flex_1().min_w_0().line_height(relative(1.3))
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).when(on, |d| d.text_color(t.ax)).child(title))
                    .child(div().truncate().text_size(px(12.)).text_color(t.tm).child(artist)))
                .when(!on, |d| d.child(icon_button(("qx", ix), t, 24., icon!("x", 14., t.tf))))
        };
        div().flex_1().min_h_0().flex().flex_col().gap(px(2.)).overflow_hidden()
            .child(div().mb(px(6.)).text_size(px(20.)).font_weight(FontWeight(650.)).child("Queue"))
            .child(label("NOW PLAYING")).child(item(99, "Window Seat, Rain", "Hoshi Yume", true))
            .child(label("NEXT FROM NIGHT BUS HOME"))
            .children(QUEUE.iter().enumerate().map(|(i, (a, b))| item(i, a, b, false)))
    }
}

impl Render for Player {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let view = self.view;

        let header = div().h(px(32.)).flex_none().flex().items_center().justify_between().gap_2().pl(px(4.))
            .child(div().truncate().font_weight(FontWeight::SEMIBOLD).text_size(px(13.)).child("Night Bus Home"))
            .child(icon_button("pl-hide", &t, 32., icon!("colr", 16., t.tm)).on_click(cx.listener(|_, _, _, cx| cx.emit(Hide))));

        let ident = div().flex_none().flex().items_center().gap(px(10.))
            .when(view != PlayerView::Art, |d| d.child(art(&t, 46., 50., false)))
            .child(div().flex_1().min_w_0().line_height(relative(1.3))
                .child(div().truncate().text_size(px(15.)).font_weight(FontWeight(650.)).child("Window Seat, Rain"))
                .child(div().text_color(t.tm).child("Hoshi Yume")))
            .child(icon_button("fav", &t, 32., icon!("star", 16., t.tm)))
            .child(icon_button("more", &t, 32., icon!("more", 16., t.tm)));

        let scrub = div().flex_none().flex().flex_col().gap(px(6.)).text_size(px(11.)).text_color(t.tf)
            .child(bar(t.s3, t.tx, 4., 0.38))
            .child(div().flex().justify_between().child("1:34").child("−2:34"));

        let transport = div().flex_none().flex().justify_between().items_center().px(px(14.)).children([
            ("prev", icon!("rw", 26., t.tx)), ("pp", icon!("play", 34., t.tx)), ("next", icon!("ff", 26., t.tx)),
        ].map(|(id, ic)| icon_button(id, &t, 52., ic)));

        let volume = div().flex_none().flex().items_center().gap(px(10.)).text_color(t.tm)
            .child(icon!("vlo", 16., t.tm)).child(div().flex_1().child(bar(t.s3, t.tm, 4., 0.7))).child(icon!("vhi", 16., t.tm));

        let mode_btn = |id: &'static str, v: PlayerView, ic: Svg| {
            div().id(id).w(px(44.)).h(px(38.)).flex().items_center().justify_center().rounded(px(11.)).cursor_pointer()
                .when(view == v, |d| d.bg(t.s3)).hover(|s| s.bg(t.s2)).active(|s| s.bg(t.s3))
                .on_click(cx.listener(move |this, _, _, cx| this.set_view(if this.view == v { PlayerView::Art } else { v }, cx)))
                .child(ic)
        };
        let mid = if view == PlayerView::Queue {
            div().h(px(38.)).flex().items_center().px(px(4.)).gap(px(2.)).rounded_full().bg(t.s3)
                .child(icon_button("shuffle", &t, 30., icon!("shuf", 16., t.tm)).w(px(44.)))
                .child(icon_button("repeat", &t, 30., icon!("rep", 16., t.tm)).w(px(44.)))
        } else {
            div().child(icon_button("cast", &t, 32., icon!("cast", 16., t.tm)))
        };
        let modes = div().flex_none().flex().justify_between().items_center()
            .child(mode_btn("m-lyrics", PlayerView::Lyrics, icon!("lyr", 16., if view == PlayerView::Lyrics { t.tx } else { t.tm })))
            .child(mid)
            .child(mode_btn("m-queue", PlayerView::Queue, icon!("que", 16., if view == PlayerView::Queue { t.tx } else { t.tm })));

        div().size_full().flex().flex_col().gap(px(14.)).pt(px(12.)).pb(px(12.)).pl(px(14.)).pr(px(18.)).overflow_hidden()
            .child(header)
            .when(view == PlayerView::Art, |d| d.child(div().w_full().h(px(WIDTH - 32.)).flex_none().rounded(px(12.)).bg(t.art(50.)).shadow_lg()))
            .child(ident)
            .map(|d| match view {
                PlayerView::Art => d.child(div().flex_1()),
                PlayerView::Lyrics => d.child(self.lyrics(&t)).child(div().flex_none().flex().justify_between().text_size(px(12.)).text_color(t.tm).child("Lyrics by YouTube Music")),
                PlayerView::Queue => d.child(self.queue(&t)),
            })
            .child(scrub).child(transport).child(volume).child(modes)
            .child(div().flex_none().text_center().text_size(px(11.)).text_color(t.tf).child("This computer"))
    }
}
