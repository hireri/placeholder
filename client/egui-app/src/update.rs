//! Looks for a newer release of the project on GitHub; one request on a worker thread, never on the UI thread.

use crate::data::Remote;
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

const API: &str = "https://api.github.com/repos/hireri/serafin/releases/latest";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Release {
    pub tag_name: String,
    pub body: String,
    pub html_url: String,
}

static LATEST: LazyLock<Remote<Release>> = LazyLock::new(Remote::new);

pub fn latest() -> &'static Remote<Release> {
    &LATEST
}

fn parts(v: &str) -> Vec<u32> {
    v.trim_start_matches('v').split(|c: char| !c.is_ascii_digit()).map_while(|p| p.parse().ok()).collect()
}

pub fn newer(tag: &str) -> bool {
    parts(tag) > parts(VERSION)
}

/// Fetches the latest release once per session (a no-op while loaded); `force` fetches again. A newer one raises a toast.
pub fn check(ctx: &eframe::egui::Context, force: bool) {
    if force {
        LATEST.reset();
    }
    LATEST.ensure(ctx, || {
        let r: Release = match ureq::get(API).set("User-Agent", "serafin").set("Accept", "application/vnd.github+json").timeout(Duration::from_secs(10)).call() {
            Ok(r) => r.into_json().map_err(|e| e.to_string())?,
            Err(ureq::Error::Status(404, _)) => return Err("No releases published yet".into()),
            Err(e) => return Err(e.to_string()),
        };
        if newer(&r.tag_name) {
            crate::toast::info(format!("Serafin {} is available", r.tag_name));
        }
        Ok(r)
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions() {
        assert!(super::parts("v1.10.0") > super::parts("1.9.3"));
        assert_eq!(super::parts("v2.0.1-beta"), [2, 0, 1]);
        assert!(!super::newer(super::VERSION));
    }
}
