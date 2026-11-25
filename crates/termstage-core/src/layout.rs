//! Milestone 1–2 layout: a centered vertical stack.
//! Replaced by `taffy` flex/grid layout (with scrolling) in Milestone 3.

use ratatui_core::layout::Rect;

/// Result of stacking widgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    /// One rect per widget; hidden widgets get an empty rect.
    pub rects: Vec<Rect>,
    /// How many widgets did not fit (they are listed last).
    pub hidden: usize,
}

/// Stack `sizes` vertically, centered in `area`, separated by `gap` rows.
///
/// If everything fits, it is centered. If not, widgets are shown from the top
/// **whole** (never squashed), the rest are hidden, and the last row is left
/// free for a "more below" hint.
pub fn center_stack(area: Rect, sizes: &[(u16, u16)], gap: u16) -> Stack {
    let total: u32 = sizes.iter().map(|&(_, h)| u32::from(h)).sum::<u32>()
        + u32::from(gap) * sizes.len().saturating_sub(1) as u32;
    let place = |x_w: u16, y: u16, h: u16| {
        let w = x_w.min(area.width);
        Rect::new(area.x + (area.width - w) / 2, y, w, h)
    };

    if total <= u32::from(area.height) {
        let mut y = area.y + (area.height - total as u16) / 2;
        let rects = sizes
            .iter()
            .map(|&(w, h)| {
                let r = place(w, y, h);
                y += h + gap;
                r
            })
            .collect();
        return Stack { rects, hidden: 0 };
    }

    let room = area.height.saturating_sub(1); // last row: the hint
    let mut rects = Vec::with_capacity(sizes.len());
    let mut y = area.y;
    let mut shown = 0;
    for (i, &(w, h)) in sizes.iter().enumerate() {
        let used = y - area.y;
        let fits = used + h <= room;
        if fits || i == 0 {
            // The first widget is always shown, clipped if it alone is too tall.
            let h = h.min(room.saturating_sub(used).max(1));
            rects.push(place(w, y, h));
            y = (y + h + gap).min(area.bottom());
            shown += 1;
        } else {
            break;
        }
    }
    let hidden = sizes.len() - shown;
    rects.extend(std::iter::repeat_n(Rect::new(area.x, area.y, 0, 0), hidden));
    Stack { rects, hidden }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_when_everything_fits() {
        let area = Rect::new(0, 0, 80, 24);
        let s = center_stack(area, &[(40, 5), (20, 1)], 1);
        assert_eq!(s.hidden, 0);
        assert_eq!(s.rects[0], Rect::new(20, 8, 40, 5));
        assert_eq!(s.rects[1], Rect::new(30, 14, 20, 1));
    }

    #[test]
    fn overflow_hides_whole_widgets_and_keeps_a_hint_row() {
        let area = Rect::new(0, 0, 80, 12);
        let s = center_stack(area, &[(40, 5), (40, 5), (40, 5)], 1);
        // Rows 0–4 and 6–10 hold two whole cards; row 11 is the hint row.
        assert_eq!(s.hidden, 1);
        assert_eq!(s.rects[0], Rect::new(20, 0, 40, 5));
        assert_eq!(s.rects[1], Rect::new(20, 6, 40, 5));
        assert!(s.rects[2].is_empty());
    }

    #[test]
    fn a_single_huge_widget_is_clipped_not_hidden() {
        let area = Rect::new(0, 0, 10, 4);
        let s = center_stack(area, &[(50, 30)], 0);
        assert_eq!(s.hidden, 0);
        assert_eq!(s.rects[0], Rect::new(0, 0, 10, 3));
    }
}
