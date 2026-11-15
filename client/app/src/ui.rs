//! Small shared building blocks.

use crate::theme::Theme;
use gpui::prelude::*;
use gpui::*;

/// Static SVG from assets/icons; the colour must be explicit (svg does not inherit text colour).
macro_rules! icon {
    ($name:literal, $size:expr, $color:expr) => {
        gpui::svg().path(concat!("icons/", $name, ".svg")).w(gpui::px($size)).h(gpui::px($size)).flex_none().text_color($color)
    };
}

/// Circular ghost button with the tonal hover/press ramp from the design.
pub fn icon_button(id: impl Into<ElementId>, t: &Theme, size: f32, icon: Svg) -> Stateful<Div> {
    div().id(id).size(px(size)).flex_none().flex().items_center().justify_center().rounded_full().cursor_pointer()
        .hover(|s| s.bg(t.s2)).active(|s| s.bg(t.s3)).child(icon)
}

/// Placeholder artwork square (or circle for artists).
pub fn art(t: &Theme, size: f32, offset: f32, round: bool) -> Div {
    let d = div().size(px(size)).flex_none().bg(t.art(offset));
    if round { d.rounded_full() } else { d.rounded(t.art_r) }
}

/// Thin horizontal bar with a filled fraction.
pub fn bar(track: Hsla, fill: Hsla, h: f32, frac: f32) -> Div {
    div().h(px(h)).w_full().rounded_full().bg(track).child(div().h_full().w(relative(frac)).rounded_full().bg(fill))
}
