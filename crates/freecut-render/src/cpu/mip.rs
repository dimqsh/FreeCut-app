//! Box-filtered mip levels, so pictures drawn much smaller than their size
//! don't alias (a single bilinear tap per output pixel skips texels once
//! the picture is minified by 2× or more).
//!
//! Level `k` halves level `k − 1` with a 2 × 2 box filter weighted by alpha
//! (sizes rounded up; the last row or column averages what exists).
//! Premultiplied pictures (text layers) average all four channels alike. The
//! compositor picks the level nearest to the layer's minification and
//! samples it bilinearly; levels are kept for the pictures still alive, so
//! a paused preview or a still image builds its levels once.

use std::sync::{Arc, Weak};

use super::Picture;

/// Pictures kept with their levels before the oldest is dropped.
const MAX_ENTRIES: usize = 32;

/// The mip level for a layer that steps `rho` source texels per output
/// pixel: the nearest power of two (`round(log2 rho)`), 0 below √2.
pub(super) fn level_for(rho: f64) -> u32 {
    if !rho.is_finite() || rho < std::f64::consts::SQRT_2 {
        return 0;
    }
    rho.log2().round().clamp(0.0, 30.0) as u32
}

/// `pic` halved (sizes rounded up) with a 2 × 2 alpha-weighted box filter.
pub(super) fn half(pic: &Picture) -> Picture {
    let (w, h) = (pic.width as usize, pic.height as usize);
    let (hw, hh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    let mut rgba = Vec::with_capacity(hw * hh * 4);
    for y in 0..hh {
        let rows = (2 * y)..(2 * y + 2).min(h);
        for x in 0..hw {
            let cols = (2 * x)..(2 * x + 2).min(w);
            let (mut sum_a, mut count) = (0u32, 0u32);
            let mut c = [0u32; 3];
            for sy in rows.clone() {
                for sx in cols.clone() {
                    let i = (sy * w + sx) * 4;
                    let a = u32::from(pic.rgba[i + 3]);
                    sum_a += a;
                    count += 1;
                    for (sum, &v) in c.iter_mut().zip(&pic.rgba[i..i + 3]) {
                        *sum += u32::from(v) * a;
                    }
                }
            }
            let px = if sum_a == 0 {
                [0, 0, 0, 0]
            } else {
                let rgb = c.map(|v| ((v + sum_a / 2) / sum_a).min(255) as u8);
                let a = ((sum_a + count / 2) / count.max(1)).min(255) as u8;
                [rgb[0], rgb[1], rgb[2], a]
            };
            rgba.extend_from_slice(&px);
        }
    }
    Picture {
        width: hw as u32,
        height: hh as u32,
        rotation: pic.rotation,
        mirrored: pic.mirrored,
        rgba,
    }
}

/// `pic` (premultiplied) halved with a plain 2 × 2 box filter.
pub(super) fn half_premultiplied(pic: &Picture) -> Picture {
    let (w, h) = (pic.width as usize, pic.height as usize);
    let (hw, hh) = (w.div_ceil(2).max(1), h.div_ceil(2).max(1));
    let mut rgba = Vec::with_capacity(hw * hh * 4);
    for y in 0..hh {
        let rows = (2 * y)..(2 * y + 2).min(h);
        for x in 0..hw {
            let cols = (2 * x)..(2 * x + 2).min(w);
            let mut c = [0u32; 4];
            let mut count = 0u32;
            for sy in rows.clone() {
                for sx in cols.clone() {
                    let i = (sy * w + sx) * 4;
                    count += 1;
                    for (sum, &v) in c.iter_mut().zip(&pic.rgba[i..i + 4]) {
                        *sum += u32::from(v);
                    }
                }
            }
            let count = count.max(1);
            rgba.extend(c.map(|v| ((v + count / 2) / count).min(255) as u8));
        }
    }
    Picture {
        width: hw as u32,
        height: hh as u32,
        rotation: pic.rotation,
        mirrored: pic.mirrored,
        rgba,
    }
}

struct Entry {
    /// Identifies the picture while it is alive (pictures are immutable
    /// behind their `Arc`).
    source: Weak<Picture>,
    /// Levels 1, 2, … built so far.
    levels: Vec<Arc<Picture>>,
}

/// Mip levels of recently drawn pictures.
#[derive(Default)]
pub(super) struct MipCache {
    entries: Vec<Entry>,
}

impl MipCache {
    /// Level `k ≥ 1` of `pic`, built on first use. Levels stop shrinking at
    /// 1 × 1. A picture is always asked for with the same `premultiplied`.
    pub fn level(&mut self, pic: &Arc<Picture>, k: u32, premultiplied: bool) -> Arc<Picture> {
        let weak = Arc::downgrade(pic);
        let index = match self.entries.iter().position(|e| e.source.ptr_eq(&weak)) {
            Some(i) => i,
            None => {
                self.entries.retain(|e| e.source.strong_count() > 0);
                if self.entries.len() >= MAX_ENTRIES {
                    self.entries.remove(0);
                }
                self.entries.push(Entry {
                    source: weak,
                    levels: Vec::new(),
                });
                self.entries.len() - 1
            }
        };
        let levels = &mut self.entries[index].levels;
        while levels.len() < k as usize {
            let prev = levels.last().unwrap_or(pic);
            if prev.width <= 1 && prev.height <= 1 {
                break;
            }
            let next = Arc::new(if premultiplied {
                half_premultiplied(prev)
            } else {
                half(prev)
            });
            levels.push(next);
        }
        levels.last().map_or_else(|| Arc::clone(pic), Arc::clone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_minification() {
        assert_eq!(level_for(1.0), 0);
        assert_eq!(level_for(1.4), 0);
        assert_eq!(level_for(1.5), 1);
        assert_eq!(level_for(2.0), 1);
        assert_eq!(level_for(3.0), 2);
        assert_eq!(level_for(f64::NAN), 0);
    }

    #[test]
    fn halving_averages_and_weights_by_alpha() {
        // 3 × 1: [white, black, red(transparent)] → 2 × 1.
        let pic = Picture::new(3, 1, vec![255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 0, 0]);
        let h = half(&pic);
        assert_eq!((h.width, h.height), (2, 1));
        assert_eq!(&h.rgba[..4], &[128, 128, 128, 255]);
        // A transparent texel alone stays transparent.
        assert_eq!(&h.rgba[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn premultiplied_halving_averages_every_channel() {
        let pic = Picture::new(2, 1, vec![200, 100, 0, 200, 0, 50, 0, 0]);
        let h = half_premultiplied(&pic);
        assert_eq!(h.rgba, [100, 75, 0, 100]);
    }

    #[test]
    fn levels_are_cached_while_the_picture_lives() {
        let pic = Arc::new(Picture::new(4, 4, vec![200; 64]));
        let mut cache = MipCache::default();
        let a = cache.level(&pic, 1, false);
        let b = cache.level(&pic, 1, false);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!((a.width, a.height), (2, 2));
        let deep = cache.level(&pic, 9, false);
        assert_eq!((deep.width, deep.height), (1, 1));
    }
}
