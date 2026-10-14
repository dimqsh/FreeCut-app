//! Color wheels (CL-015): Shadows, Midtones, Highlights and Offset, each a
//! puck on a disk plus a luminance level, and one Strength for all four.
//!
//! Pucks are `i16` x/y in `−1000..=1000` with x² + y² ≤ 1000² (CL-001's
//! model notes); [`Wheel::new`] pulls a puck that lies outside the disk
//! back onto its rim, in integers, so the invariant holds for every
//! [`Wheel`]. The colour math lives in `freecut-render`.

/// Which wheel (CL-015), in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WheelId {
    Shadows,
    Midtones,
    Highlights,
    Offset,
}

impl WheelId {
    pub const ALL: [WheelId; 4] = [
        WheelId::Shadows,
        WheelId::Midtones,
        WheelId::Highlights,
        WheelId::Offset,
    ];

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            WheelId::Shadows => "shadows",
            WheelId::Midtones => "midtones",
            WheelId::Highlights => "highlights",
            WheelId::Offset => "offset",
        }
    }

    pub fn parse(s: &str) -> Option<WheelId> {
        WheelId::ALL.into_iter().find(|w| w.as_str() == s)
    }
}

/// The disk's radius in puck units.
pub const PUCK_RADIUS: i16 = 1000;

/// `(x, y)` pulled onto the disk: unchanged inside it, otherwise moved
/// along the same direction to the rim (a point just inside it, with
/// integer coordinates).
pub fn on_disk(x: i16, y: i16) -> (i16, i16) {
    let r = i64::from(PUCK_RADIUS);
    let (x, y) = (i64::from(x), i64::from(y));
    let r2 = x * x + y * y;
    if r2 <= r * r {
        return (x as i16, y as i16);
    }
    let len = r2.isqrt().max(1);
    let (mut nx, mut ny) = (x * r / len, y * r / len);
    // The floor square root can leave the point a unit outside the rim.
    while nx * nx + ny * ny > r * r {
        if nx.abs() >= ny.abs() {
            nx -= nx.signum();
        } else {
            ny -= ny.signum();
        }
    }
    // Within ±1000 here, so the conversions are exact.
    (
        i16::try_from(nx).unwrap_or(0),
        i16::try_from(ny).unwrap_or(0),
    )
}

/// One wheel: the puck (x along the blue colour difference, y along the
/// red one, as on a vectorscope) and its luminance level −100..=+100.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Wheel {
    x: i16,
    y: i16,
    luma: i16,
}

impl Wheel {
    /// A wheel with its puck pulled onto the disk and its luminance
    /// limited to ±100.
    pub fn new(x: i16, y: i16, luma: i16) -> Wheel {
        let (x, y) = on_disk(x, y);
        Wheel {
            x,
            y,
            luma: luma.clamp(-100, 100),
        }
    }

    pub fn x(&self) -> i16 {
        self.x
    }

    pub fn y(&self) -> i16 {
        self.y
    }

    pub fn luma(&self) -> i16 {
        self.luma
    }

    /// The same wheel with the puck at `(x, y)`, pulled onto the disk.
    #[must_use]
    pub fn with_puck(self, x: i16, y: i16) -> Wheel {
        Wheel::new(x, y, self.luma)
    }

    #[must_use]
    pub fn with_luma(self, luma: i16) -> Wheel {
        Wheel::new(self.x, self.y, luma)
    }

    /// Whether the puck sits at the centre.
    pub fn puck_is_centred(&self) -> bool {
        self.x == 0 && self.y == 0
    }

    /// Whether the wheel changes nothing.
    pub fn is_default(&self) -> bool {
        *self == Wheel::default()
    }
}

/// The Wheels section (CL-015): four wheels and the Strength (0–100 %,
/// 100 by default) that scales all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorWheels {
    pub shadows: Wheel,
    pub midtones: Wheel,
    pub highlights: Wheel,
    pub offset: Wheel,
    pub strength: u8,
}

impl Default for ColorWheels {
    fn default() -> Self {
        ColorWheels {
            shadows: Wheel::default(),
            midtones: Wheel::default(),
            highlights: Wheel::default(),
            offset: Wheel::default(),
            strength: 100,
        }
    }
}

impl ColorWheels {
    pub fn wheel(&self, id: WheelId) -> &Wheel {
        match id {
            WheelId::Shadows => &self.shadows,
            WheelId::Midtones => &self.midtones,
            WheelId::Highlights => &self.highlights,
            WheelId::Offset => &self.offset,
        }
    }

    pub fn wheel_mut(&mut self, id: WheelId) -> &mut Wheel {
        match id {
            WheelId::Shadows => &mut self.shadows,
            WheelId::Midtones => &mut self.midtones,
            WheelId::Highlights => &mut self.highlights,
            WheelId::Offset => &mut self.offset,
        }
    }

    /// Whether any wheel moves a colour or a level (Strength aside).
    pub fn any_set(&self) -> bool {
        WheelId::ALL.iter().any(|w| !self.wheel(*w).is_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pucks_outside_the_disk_land_on_its_rim() {
        assert_eq!(on_disk(300, -400), (300, -400));
        assert_eq!(on_disk(2000, 0), (1000, 0));
        assert_eq!(on_disk(0, i16::MIN), (0, -1000));
        for (x, y) in [
            (i16::MAX, i16::MAX),
            (i16::MIN, 1),
            (999, 999),
            (-708, 708),
            (1001, 1),
        ] {
            let (nx, ny) = on_disk(x, y);
            let r2 = i32::from(nx).pow(2) + i32::from(ny).pow(2);
            assert!(r2 <= 1_000_000, "({x}, {y}) → ({nx}, {ny})");
            assert!(r2 >= 995_000, "on the rim: ({nx}, {ny})");
            // Same quadrant.
            assert!(i32::from(nx).signum() * i32::from(x).signum() >= 0);
            assert!(i32::from(ny).signum() * i32::from(y).signum() >= 0);
        }
        let w = Wheel::new(5000, 0, 400);
        assert_eq!((w.x(), w.y(), w.luma()), (1000, 0, 100));
        assert!(ColorWheels::default().wheel(WheelId::Offset).is_default());
        assert_eq!(ColorWheels::default().strength, 100);
    }
}
