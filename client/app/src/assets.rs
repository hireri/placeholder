use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Icons are compiled in; `icon!("play")` resolves to `icons/play.svg`.
const ICONS: &[(&str, &[u8])] = &[
    ("icons/alb.svg", include_bytes!("../assets/icons/alb.svg")),
    ("icons/art.svg", include_bytes!("../assets/icons/art.svg")),
    ("icons/back.svg", include_bytes!("../assets/icons/back.svg")),
    ("icons/bell.svg", include_bytes!("../assets/icons/bell.svg")),
    ("icons/cast.svg", include_bytes!("../assets/icons/cast.svg")),
    ("icons/col.svg", include_bytes!("../assets/icons/col.svg")),
    ("icons/colr.svg", include_bytes!("../assets/icons/colr.svg")),
    ("icons/dir.svg", include_bytes!("../assets/icons/dir.svg")),
    ("icons/ff.svg", include_bytes!("../assets/icons/ff.svg")),
    ("icons/fwd.svg", include_bytes!("../assets/icons/fwd.svg")),
    ("icons/grip.svg", include_bytes!("../assets/icons/grip.svg")),
    ("icons/hist.svg", include_bytes!("../assets/icons/hist.svg")),
    ("icons/home.svg", include_bytes!("../assets/icons/home.svg")),
    ("icons/imp.svg", include_bytes!("../assets/icons/imp.svg")),
    ("icons/lib.svg", include_bytes!("../assets/icons/lib.svg")),
    ("icons/lyr.svg", include_bytes!("../assets/icons/lyr.svg")),
    ("icons/more.svg", include_bytes!("../assets/icons/more.svg")),
    ("icons/pl.svg", include_bytes!("../assets/icons/pl.svg")),
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("icons/que.svg", include_bytes!("../assets/icons/que.svg")),
    ("icons/rep.svg", include_bytes!("../assets/icons/rep.svg")),
    ("icons/rw.svg", include_bytes!("../assets/icons/rw.svg")),
    ("icons/search.svg", include_bytes!("../assets/icons/search.svg")),
    ("icons/shuf.svg", include_bytes!("../assets/icons/shuf.svg")),
    ("icons/song.svg", include_bytes!("../assets/icons/song.svg")),
    ("icons/star.svg", include_bytes!("../assets/icons/star.svg")),
    ("icons/vhi.svg", include_bytes!("../assets/icons/vhi.svg")),
    ("icons/vlo.svg", include_bytes!("../assets/icons/vlo.svg")),
    ("icons/x.svg", include_bytes!("../assets/icons/x.svg"))
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(p, _)| SharedString::from(*p)).collect())
    }
}
