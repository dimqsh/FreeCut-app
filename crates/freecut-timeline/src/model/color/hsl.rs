//! HSL per hue band (CL-013): eight bands, each with a hue shift, a
//! saturation and a lightness level. The band membership math lives in
//! `freecut-render`.

/// One of the eight hue bands of CL-013, in swatch order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HueBand {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Magenta,
}

impl HueBand {
    pub const ALL: [HueBand; 8] = [
        HueBand::Red,
        HueBand::Orange,
        HueBand::Yellow,
        HueBand::Green,
        HueBand::Cyan,
        HueBand::Blue,
        HueBand::Purple,
        HueBand::Magenta,
    ];

    /// The band's place in [`HueBand::ALL`] and in [`Hsl::bands`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The band's centre on the hue circle, in degrees (CL-013).
    pub const fn center_degrees(self) -> u16 {
        match self {
            HueBand::Red => 0,
            HueBand::Orange => 30,
            HueBand::Yellow => 60,
            HueBand::Green => 120,
            HueBand::Cyan => 180,
            HueBand::Blue => 240,
            HueBand::Purple => 270,
            HueBand::Magenta => 300,
        }
    }

    /// The stable name in project files.
    pub const fn as_str(self) -> &'static str {
        match self {
            HueBand::Red => "red",
            HueBand::Orange => "orange",
            HueBand::Yellow => "yellow",
            HueBand::Green => "green",
            HueBand::Cyan => "cyan",
            HueBand::Blue => "blue",
            HueBand::Purple => "purple",
            HueBand::Magenta => "magenta",
        }
    }

    pub fn parse(s: &str) -> Option<HueBand> {
        HueBand::ALL.into_iter().find(|b| b.as_str() == s)
    }
}

/// One of a band's three sliders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HslChannel {
    Hue,
    Saturation,
    Lightness,
}

impl HslChannel {
    pub const ALL: [HslChannel; 3] = [
        HslChannel::Hue,
        HslChannel::Saturation,
        HslChannel::Lightness,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            HslChannel::Hue => "hue",
            HslChannel::Saturation => "saturation",
            HslChannel::Lightness => "lightness",
        }
    }
}

/// A band's levels, each −100..=+100 (CL-013): Hue (±30°), Saturation
/// (−100 removes the band's colour) and Lightness (±50 %).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HslBand {
    pub hue: i16,
    pub sat: i16,
    pub light: i16,
}

impl HslBand {
    pub fn get(&self, channel: HslChannel) -> i16 {
        match channel {
            HslChannel::Hue => self.hue,
            HslChannel::Saturation => self.sat,
            HslChannel::Lightness => self.light,
        }
    }

    pub fn get_mut(&mut self, channel: HslChannel) -> &mut i16 {
        match channel {
            HslChannel::Hue => &mut self.hue,
            HslChannel::Saturation => &mut self.sat,
            HslChannel::Lightness => &mut self.light,
        }
    }

    /// Whether the band changes nothing.
    pub fn is_default(&self) -> bool {
        *self == HslBand::default()
    }
}

/// The HSL section (CL-013): one [`HslBand`] per [`HueBand`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Hsl {
    pub bands: [HslBand; 8],
}

impl Hsl {
    pub fn band(&self, band: HueBand) -> &HslBand {
        &self.bands[band.index()]
    }

    pub fn band_mut(&mut self, band: HueBand) -> &mut HslBand {
        &mut self.bands[band.index()]
    }

    /// Whether every band is at its defaults.
    pub fn is_default(&self) -> bool {
        self.bands.iter().all(HslBand::is_default)
    }
}
