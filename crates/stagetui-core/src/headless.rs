//! Headless rendering: what agents and CI see.
//!
//! A snapshot gives the screen as **plain text in reading order** (best for
//! LLMs), the **JSON widget tree** (ids, types, bounds, state), and on request
//! a **PNG** drawn exactly like a good terminal would.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::Result;
use crate::frame::Frame;
use crate::session::Session;

/// One headless capture.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub cols: u16,
    pub rows: u16,
    pub text: String,
    pub tree: Value,
}

fn frame(session: &mut Session, cols: u16, rows: u16) -> (u16, u16, Frame) {
    let (c, r) = (cols.max(1), rows.max(1));
    (c, r, session.frame(c, r))
}

/// Render `session` offscreen at `cols × rows`.
pub fn snapshot(session: &mut Session, cols: u16, rows: u16) -> Snapshot {
    let (c, r, f) = frame(session, cols, rows);
    Snapshot {
        cols: c,
        rows: r,
        text: f.text(),
        tree: session.tree_json(c, r),
    }
}

/// Render `session` offscreen to PNG bytes.
pub fn png(session: &mut Session, cols: u16, rows: u16) -> Result<Vec<u8>> {
    let (_, _, f) = frame(session, cols, rows);
    crate::gfx::png(&crate::gfx::raster::render(&f, session.cell_px()))
}

/// Deterministic time for headless runs: animations and timers advance only
/// when the script says so, so snapshots are byte-identical across runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VirtualClock {
    now: Duration,
}

impl VirtualClock {
    pub fn now(&self) -> Duration {
        self.now
    }

    pub fn advance(&mut self, by: Duration) {
        self.now += by;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = r#"{
        "theme": "aurora-dark",
        "children": [
            {"type": "card", "content": "Welcome to stagetui.\nPress q or Esc to quit.", "props": {"id": "hello", "title": "Hello", "icon": "sparkles"}},
            {"type": "card", "content": "مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج", "props": {"id": "arabic", "title": "عربي", "accent": "mint"}},
            {"type": "text", "content": "こんにちは · नमस्ते · Hello مرحبا"}
        ]
    }"#;

    #[test]
    fn deterministic_and_golden() {
        let mut s = Session::from_json(HELLO).unwrap();
        let a = snapshot(&mut s, 60, 20);
        let b = snapshot(&mut Session::from_json(HELLO).unwrap(), 60, 20);
        assert_eq!(a.text, b.text);
        assert_eq!(a.tree, b.tree);

        let golden_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/snapshots/hello_60x20.txt"
        );
        if std::env::var("STAGETUI_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
            std::fs::write(golden_path, &a.text).unwrap();
        }
        let golden = std::fs::read_to_string(golden_path)
            .expect("missing golden snapshot; run with STAGETUI_UPDATE_SNAPSHOTS=1 to create it");
        assert_eq!(
            a.text, golden,
            "snapshot changed; review and re-run with STAGETUI_UPDATE_SNAPSHOTS=1"
        );
    }

    #[test]
    fn arabic_reads_in_logical_order() {
        let mut s = Session::from_json(HELLO).unwrap();
        let snap = snapshot(&mut s, 60, 20);
        assert!(
            snap.text.contains("مَرْحَباً بِكُمْ فِي تِيرْم سْتِيج"),
            "{}",
            snap.text
        );
        assert!(snap.text.contains("عربي"));
    }

    #[test]
    fn tree_has_ids_bounds_and_direction() {
        let mut s = Session::from_json(HELLO).unwrap();
        let snap = snapshot(&mut s, 60, 20);
        let kids = snap.tree["children"].as_array().unwrap();
        assert_eq!(kids[0]["id"], "hello");
        assert_eq!(kids[1]["dir"], "rtl");
        assert_eq!(kids[2]["id"], "text-1");
        let b = &kids[0]["bounds"];
        assert!(b["w"].as_u64().unwrap() > 0 && b["h"].as_u64().unwrap() > 0);
        assert!(snap.text.contains("Hello") && snap.text.contains("Welcome to stagetui."));
    }

    #[test]
    fn png_has_the_right_size() {
        let mut s = Session::from_json(HELLO).unwrap();
        let bytes = png(&mut s, 30, 10).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        let img = crate::gfx::load::from_bytes(&bytes, "snap.png").unwrap();
        assert_eq!((img.width, img.height), (300, 200));
    }

    #[test]
    fn tiny_and_huge_grids_do_not_panic() {
        let mut s = Session::from_json(HELLO).unwrap();
        for (c, r) in [(1, 1), (2, 2), (5, 3), (300, 100), (0, 0)] {
            let snap = snapshot(&mut s, c, r);
            assert!(snap.text.lines().count() <= usize::from(r.max(1)));
        }
    }

    #[test]
    fn too_small_windows_scroll_instead_of_squashing() {
        let mut s = Session::from_json(HELLO).unwrap();
        let snap = snapshot(&mut s, 60, 9);
        assert_eq!(snap.tree["scroll"]["offset"], 0, "{}", snap.tree);
        assert!(snap.tree["scroll"]["content_rows"].as_u64().unwrap() > 9);
        let kids = snap.tree["children"].as_array().unwrap();
        assert_eq!(kids[0]["on_screen"], true);
        assert_eq!(kids[0]["bounds"]["h"], 6, "the first card is whole");
        assert!(
            snap.text.contains('┃'),
            "a scrollbar shows there is more:\n{}",
            snap.text
        );
    }

    #[test]
    fn virtual_clock() {
        let mut c = VirtualClock::default();
        c.advance(Duration::from_millis(16));
        assert_eq!(c.now(), Duration::from_millis(16));
    }
}
