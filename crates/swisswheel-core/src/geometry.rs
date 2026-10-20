//! Wheel layout: which sector a point falls in, and where to place the wheel.
//!
//! Radii are in DIPs (scale 1.0); callers pass the monitor's DPI scale.
//! Angles are degrees, 0 at 12 o'clock, increasing clockwise.

pub const OUTER_RADIUS: f32 = 160.0;
pub const CANCEL_RADIUS: f32 = 48.0;
pub const RING_SPLIT_RADIUS: f32 = 104.0;
pub const RING_CAPACITY: usize = 12;
pub const MAX_ITEMS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sector {
    pub inner_radius: f32,
    pub outer_radius: f32,
    /// May be negative: sector 0 straddles 12 o'clock.
    pub start_deg: f32,
    pub end_deg: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Cancel,
    Item(usize),
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// A ring of `count` equal sectors whose first item has index `first`.
struct Ring {
    inner: f32,
    outer: f32,
    first: usize,
    count: usize,
}

fn rings(item_count: usize) -> (Ring, Option<Ring>) {
    let n = item_count.min(MAX_ITEMS);
    if n <= RING_CAPACITY {
        (
            Ring {
                inner: CANCEL_RADIUS,
                outer: OUTER_RADIUS,
                first: 0,
                count: n,
            },
            None,
        )
    } else {
        (
            Ring {
                inner: CANCEL_RADIUS,
                outer: RING_SPLIT_RADIUS,
                first: 0,
                count: RING_CAPACITY,
            },
            Some(Ring {
                inner: RING_SPLIT_RADIUS,
                outer: OUTER_RADIUS,
                first: RING_CAPACITY,
                count: n - RING_CAPACITY,
            }),
        )
    }
}

/// Shape of sector `index` among `item_count` items, scaled to physical pixels.
pub fn sector(item_count: usize, index: usize, scale: f32) -> Sector {
    let (inner, outer) = rings(item_count);
    let ring = match outer {
        Some(o) if index >= o.first => o,
        _ => inner,
    };
    let width = 360.0 / ring.count as f32;
    let start_deg = (index - ring.first) as f32 * width - width / 2.0;
    Sector {
        inner_radius: ring.inner * scale,
        outer_radius: ring.outer * scale,
        start_deg,
        end_deg: start_deg + width,
    }
}

/// `dx, dy`: offset from the wheel center in physical pixels, y pointing down.
pub fn hit_test(item_count: usize, dx: f32, dy: f32, scale: f32) -> Hit {
    let r = dx.hypot(dy) / scale;
    if r > OUTER_RADIUS {
        return Hit::Outside;
    }
    if r < CANCEL_RADIUS || item_count == 0 {
        return Hit::Cancel;
    }
    let (inner, outer) = rings(item_count);
    let ring = match outer {
        Some(o) if r >= o.inner => o,
        _ => inner,
    };
    let theta = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
    let width = 360.0 / ring.count as f32;
    let j = (((theta + width / 2.0) % 360.0) / width) as usize;
    Hit::Item(ring.first + j.min(ring.count - 1))
}

/// Move `cursor` so a wheel of `OUTER_RADIUS * scale` fits inside `work_area`.
pub fn clamp_center(cursor: (i32, i32), work_area: Rect, scale: f32) -> (i32, i32) {
    let r = (OUTER_RADIUS * scale).ceil() as i32;
    let clamp = |v: i32, lo: i32, hi: i32| {
        if lo + r > hi - r {
            (lo + hi) / 2 // work area narrower than the wheel: center it
        } else {
            v.clamp(lo + r, hi - r)
        }
    };
    (
        clamp(cursor.0, work_area.left, work_area.right),
        clamp(cursor.1, work_area.top, work_area.bottom),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f32 = 1.0;

    #[test]
    fn center_and_outside() {
        assert_eq!(hit_test(6, 0.0, 0.0, S), Hit::Cancel);
        assert_eq!(hit_test(6, 0.0, -170.0, S), Hit::Outside);
        assert_eq!(hit_test(0, 0.0, -100.0, S), Hit::Cancel);
    }

    #[test]
    fn single_ring_directions() {
        // 4 items: top(0) right(1) bottom(2) left(3)
        assert_eq!(hit_test(4, 0.0, -100.0, S), Hit::Item(0));
        assert_eq!(hit_test(4, 100.0, 0.0, S), Hit::Item(1));
        assert_eq!(hit_test(4, 0.0, 100.0, S), Hit::Item(2));
        assert_eq!(hit_test(4, -100.0, 0.0, S), Hit::Item(3));
    }

    #[test]
    fn boundaries_and_seam() {
        assert_eq!(hit_test(4, 0.0, -48.0, S), Hit::Item(0)); // cancel-circle edge belongs to the item
        assert_eq!(hit_test(4, 0.0, -160.0, S), Hit::Item(0)); // outer edge belongs to the item
        // just left of 12 o'clock (359.9 deg) is still item 0
        let a = 359.9_f32.to_radians();
        assert_eq!(
            hit_test(4, 100.0 * a.sin(), -100.0 * a.cos(), S),
            Hit::Item(0)
        );
        // with 4 items the boundary is 45 deg; exactly 45 is float-fragile, so check both sides
        let b = 44.5_f32.to_radians();
        assert_eq!(
            hit_test(4, 100.0 * b.sin(), -100.0 * b.cos(), S),
            Hit::Item(0)
        );
        let c = 45.5_f32.to_radians();
        assert_eq!(
            hit_test(4, 100.0 * c.sin(), -100.0 * c.cos(), S),
            Hit::Item(1)
        );
    }

    #[test]
    fn two_rings() {
        // 14 items: 12 inner, 2 outer (12 top, 13 bottom)
        assert_eq!(hit_test(14, 0.0, -80.0, S), Hit::Item(0));
        assert_eq!(hit_test(14, 0.0, -104.0, S), Hit::Item(12)); // radius 104 is the outer ring
        assert_eq!(hit_test(14, 0.0, 130.0, S), Hit::Item(13));
        assert_eq!(hit_test(30, 0.0, -130.0, S), Hit::Item(12)); // capped at 24
    }

    #[test]
    fn dpi_scale() {
        assert_eq!(hit_test(4, 0.0, -230.0, 1.5), Hit::Item(0)); // 230/1.5 ~ 153
        assert_eq!(hit_test(4, 0.0, -245.0, 1.5), Hit::Outside); // ~ 163
        assert_eq!(hit_test(4, 0.0, -70.0, 1.5), Hit::Cancel); // ~ 47
    }

    #[test]
    fn sector_shape() {
        let s = sector(4, 0, 1.0);
        assert_eq!(
            (s.inner_radius, s.outer_radius, s.start_deg, s.end_deg),
            (48.0, 160.0, -45.0, 45.0)
        );
        let t = sector(14, 13, 2.0);
        assert_eq!(
            (t.inner_radius, t.outer_radius, t.start_deg, t.end_deg),
            (208.0, 320.0, 90.0, 270.0)
        );
    }

    #[test]
    fn clamp_to_work_area() {
        let wa = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert_eq!(clamp_center((960, 500), wa, 1.0), (960, 500));
        assert_eq!(clamp_center((10, 1035), wa, 1.0), (160, 880));
        assert_eq!(clamp_center((1919, 0), wa, 1.5), (1680, 240));
        // secondary monitor with negative coordinates
        let left = Rect {
            left: -1280,
            top: 0,
            right: 0,
            bottom: 1024,
        };
        assert_eq!(clamp_center((-1275, 512), left, 1.0), (-1120, 512));
    }
}
