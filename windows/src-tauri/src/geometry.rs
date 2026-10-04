//! Keep the pet window on a screen (port of the macOS dev fix c9a36dd:
//! "keep a dragged pet on screen when dropped past an edge"). Pure math on
//! physical-pixel rects so it can be unit-tested; lib.rs feeds it the
//! monitors' work areas.

/// (x, y, width, height) in physical pixels.
pub type Rect = (i32, i32, i32, i32);

/// Index of the rect closest to `p` (0 distance when inside), for a pet
/// dropped outside every screen. None only when `rects` is empty.
pub fn nearest_rect_index(p: (i32, i32), rects: &[Rect]) -> Option<usize> {
    let dist = |r: &Rect| {
        let (x, y, w, h) = *r;
        let dx = (x - p.0).max(0).max(p.0 - (x + w)) as i64;
        let dy = (y - p.1).max(0).max(p.1 - (y + h)) as i64;
        dx * dx + dy * dy
    };
    (0..rects.len()).min_by_key(|&i| dist(&rects[i]))
}

/// Origin that keeps a `size` window inside `area`. Top/left are fitted
/// first, then bottom/right win, so for a window bigger than the screen the
/// bottom (where the pet sprite sits) stays visible (macOS verticalOrigin).
pub fn clamp_into(origin: (i32, i32), size: (i32, i32), area: Rect) -> (i32, i32) {
    let (ax, ay, aw, ah) = area;
    let mut x = origin.0;
    let mut y = origin.1;
    if x < ax { x = ax; }
    if x + size.0 > ax + aw { x = ax + aw - size.0; }
    if y < ay { y = ay; }
    if y + size.1 > ay + ah { y = ay + ah - size.1; }
    (x, y)
}

/// Where the window should be: unchanged when it is already fully on some
/// screen, otherwise clamped into the screen nearest its centre.
pub fn keep_on_screen(origin: (i32, i32), size: (i32, i32), areas: &[Rect]) -> (i32, i32) {
    let centre = (origin.0 + size.0 / 2, origin.1 + size.1 / 2);
    let Some(i) = nearest_rect_index(centre, areas) else { return origin };
    clamp_into(origin, size, areas[i])
}

/// Which side of the pet the stats card sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// Card above the pet, arrow on the card's bottom edge.
    Above,
    /// Card below the pet, arrow on the card's top edge.
    Below,
    /// No vertical room: card beside the pet, no arrow.
    Side,
}

impl Edge {
    pub fn as_str(self) -> &'static str {
        match self {
            Edge::Above => "above",
            Edge::Below => "below",
            Edge::Side => "side",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub edge: Edge,
    /// Arrow x inside the card (card-relative px), pointing at the pet centre.
    pub arrow: i32,
    /// Tallest the card may be here; the card scrolls when its content is taller.
    pub max_h: i32,
}

/// Gap between the pet and the card, room for the arrow.
pub const CARD_GAP: i32 = 6;
/// Below this much room the card goes beside the pet instead of shrinking.
pub const MIN_CARD_H: i32 = 260;

/// Where the stats card goes so it never covers the pet (macOS NSPopover on
/// the pet): above when it fits, else below, else on whichever of the two has
/// more room with the card capped to that room (it scrolls), else beside the
/// pet. `pet` is the pet's visible content (sprite + bubble) in screen px,
/// `area` the monitor's work area. Horizontally centred on the pet, clamped.
pub fn place_card(card: (i32, i32), pet: Rect, area: Rect) -> Placement {
    let (cw, ch) = card;
    let (px, py, pw, ph) = pet;
    let (ax, ay, aw, ah) = area;
    let centre = px + pw / 2;
    let x = (centre - cw / 2).max(ax).min(ax + aw - cw);
    let arrow = (centre - x).clamp(16, (cw - 16).max(16));
    let room_above = py - CARD_GAP - ay;
    let below = py + ph + CARD_GAP;
    let room_below = ay + ah - below;
    let above_at = |h: i32| Placement { x, y: py - CARD_GAP - h, edge: Edge::Above, arrow, max_h: h };
    let below_at = |h: i32| Placement { x, y: below, edge: Edge::Below, arrow, max_h: h };
    if ch <= room_above {
        return above_at(ch);
    }
    if ch <= room_below {
        return below_at(ch);
    }
    let room = room_above.max(room_below);
    if room >= MIN_CARD_H {
        return if room_below >= room_above { below_at(room_below) } else { above_at(room_above) };
    }
    // Pet in the middle of a very short screen: beside it on the roomier side.
    let right_room = ax + aw - (px + pw);
    let left_room = px - ax;
    let sx = if right_room >= left_room { px + pw + CARD_GAP } else { px - CARD_GAP - cw };
    let sx = sx.max(ax).min(ax + aw - cw);
    let h = ch.min(ah);
    let sy = (py + ph / 2 - h / 2).max(ay).min(ay + ah - h);
    Placement { x: sx, y: sy, edge: Edge::Side, arrow: 0, max_h: h }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two monitors like the user's: 1600x900 primary + 1600x900 to its right.
    const MONS: [Rect; 2] = [(0, 0, 1600, 900), (1600, 0, 1600, 900)];
    const PET: (i32, i32) = (260, 320);

    #[test]
    fn on_screen_window_is_untouched() {
        assert_eq!(keep_on_screen((1200, 500), PET, &MONS), (1200, 500));
        assert_eq!(keep_on_screen((1700, 0), PET, &MONS), (1700, 0));
    }

    #[test]
    fn dropped_low_comes_back_up() {
        // Half the sprite below the bottom edge (the macOS bug report).
        assert_eq!(keep_on_screen((400, 750), PET, &MONS), (400, 900 - 320));
    }

    #[test]
    fn dropped_past_the_right_edge_of_the_last_screen() {
        assert_eq!(keep_on_screen((3100, 100), PET, &MONS), (3200 - 260, 100));
    }

    #[test]
    fn dropped_outside_every_screen_uses_the_nearest() {
        assert_eq!(keep_on_screen((-900, -900), PET, &MONS), (0, 0));
        assert_eq!(keep_on_screen((5000, 2000), PET, &MONS), (3200 - 260, 900 - 320));
    }

    #[test]
    fn straddling_two_screens_settles_on_the_one_holding_the_centre() {
        // Centre at x=1650 is on the right monitor.
        assert_eq!(keep_on_screen((1520, 100), PET, &MONS), (1600, 100));
    }

    #[test]
    fn window_taller_than_screen_keeps_the_bottom() {
        assert_eq!(clamp_into((0, -50), (260, 1000), (0, 0, 1600, 900)), (0, 900 - 1000));
    }

    #[test]
    fn no_monitors_leaves_origin() {
        assert_eq!(keep_on_screen((5, 6), PET, &[]), (5, 6));
        assert_eq!(nearest_rect_index((0, 0), &[]), None);
    }

    const CARD: (i32, i32) = (332, 596);
    const RIGHT: Rect = (1600, 0, 1600, 900);

    fn overlaps(p: &Placement, card: (i32, i32), pet: Rect) -> bool {
        let (px, py, pw, ph) = pet;
        let h = card.1.min(p.max_h);
        p.x < px + pw && p.x + card.0 > px && p.y < py + ph && p.y + h > py
    }

    #[test]
    fn pet_at_top_puts_card_below_with_arrow_on_pet() {
        // The user's layout: pet at the top-right, content 151..320 of a 260x320 window.
        // 596 px card, 574 px of room below: below, capped to the room (scrolls).
        let pet = (2873 + 31, 151, 198, 169);
        let p = place_card(CARD, pet, RIGHT);
        assert_eq!(p.edge, Edge::Below);
        assert_eq!(p.y, 320 + CARD_GAP);
        assert_eq!(p.max_h, 900 - (320 + CARD_GAP));
        assert!(!overlaps(&p, CARD, pet));
        assert!(p.x + CARD.0 <= 3200, "stays on the monitor");
        assert_eq!(p.x + p.arrow, 2873 + 31 + 99, "arrow points at the pet centre");
    }

    #[test]
    fn short_card_fits_below_uncapped() {
        let p = place_card((332, 400), (2904, 151, 198, 169), RIGHT);
        assert_eq!((p.edge, p.max_h), (Edge::Below, 400));
    }

    #[test]
    fn pet_low_on_screen_puts_card_above() {
        let pet = (2000, 700, 198, 169);
        let p = place_card(CARD, pet, RIGHT);
        assert_eq!(p.edge, Edge::Above);
        assert_eq!(p.y + CARD.1 + CARD_GAP, 700);
        assert!(!overlaps(&p, CARD, pet));
    }

    #[test]
    fn pet_mid_screen_takes_the_roomier_side_capped() {
        let pet = (2000, 300, 198, 169);
        let p = place_card(CARD, pet, RIGHT);
        assert_eq!(p.edge, Edge::Below);
        assert_eq!(p.max_h, 900 - (469 + CARD_GAP));
        assert!(!overlaps(&p, CARD, pet));
    }

    #[test]
    fn very_short_screen_goes_beside() {
        let area = (0, 0, 1600, 500);
        let pet = (600, 200, 198, 169);
        let p = place_card(CARD, pet, area);
        assert_eq!(p.edge, Edge::Side);
        assert!(!overlaps(&p, CARD, pet));
        assert!(p.y >= 0 && p.y + p.max_h <= 500);
    }

    #[test]
    fn pet_at_left_edge_clamps_card_and_keeps_arrow_inside() {
        let pet = (0, 0, 198, 169);
        let p = place_card(CARD, pet, (0, 0, 1600, 900));
        assert_eq!(p.x, 0);
        assert!(p.arrow >= 16 && p.arrow <= CARD.0 - 16);
    }
}
