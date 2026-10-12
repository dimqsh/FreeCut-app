#![allow(dead_code)]

pub mod project;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use freecut_render::cpu::{LayerImages, Picture, TextLayer};
use freecut_timeline::glam::DMat3;
use freecut_timeline::ids::{ClipId, MediaId, TrackId};
use freecut_timeline::plan::{BlendMode, CanvasSpec, FramePlan, LayerPlan, LayerSource, PlanNode};
use freecut_timeline::time::Tick;

pub const BG: [u8; 4] = [0, 0, 0, 255];
pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const GREEN: [u8; 4] = [0, 255, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];
pub const WHITE: [u8; 4] = [255, 255, 255, 255];

pub fn media(n: u64) -> MediaId {
    MediaId::from_raw(NonZeroU64::new(n).unwrap())
}

pub fn clip(n: u64) -> ClipId {
    ClipId::from_raw(NonZeroU64::new(n).unwrap())
}

pub fn plan(width: u32, height: u32, layers: Vec<LayerPlan>) -> FramePlan {
    FramePlan {
        time: Tick::ZERO,
        frame: Tick::from_raw(705_600_000 / 30),
        frame_index: 0,
        canvas: CanvasSpec {
            width,
            height,
            background: BG,
        },
        nodes: layers.into_iter().map(PlanNode::Layer).collect(),
    }
}

pub fn layer(source: LayerSource, display: (u32, u32), m: DMat3) -> LayerPlan {
    LayerPlan {
        clip: clip(1),
        track: TrackId::from_raw(NonZeroU64::new(1).unwrap()),
        source,
        display_size: display,
        layer_to_canvas: m,
        opacity: 1.0,
        blend: BlendMode::Normal,
        motion: None,
        ligatures: true,
        color: None,
    }
}

pub fn still(n: u64) -> LayerSource {
    LayerSource::Still { asset: media(n) }
}

/// A picture from rows of pixels.
pub fn picture(rows: &[&[[u8; 4]]]) -> Picture {
    let height = rows.len() as u32;
    let width = rows.first().map_or(0, |r| r.len()) as u32;
    let rgba = rows
        .iter()
        .flat_map(|r| r.iter().flatten().copied())
        .collect();
    Picture::new(width, height, rgba)
}

pub fn solid(width: u32, height: u32, color: [u8; 4]) -> Picture {
    let rgba = (0..width * height).flat_map(|_| color).collect();
    Picture::new(width, height, rgba)
}

/// A deterministic test pattern.
pub fn pattern(width: u32, height: u32) -> Picture {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            rgba.extend_from_slice(&[
                (x * 7 + y * 3) as u8,
                (x * 13) as u8 ^ (y as u8),
                (y * 11) as u8,
                if (x / 8 + y / 8) % 3 == 0 { 128 } else { 255 },
            ]);
        }
    }
    Picture::new(width, height, rgba)
}

/// Pictures and texts held in memory.
#[derive(Default)]
pub struct Images {
    pub pictures: HashMap<MediaId, Arc<Picture>>,
    pub texts: HashMap<ClipId, TextLayer>,
    pub video_requests: Vec<(MediaId, Tick)>,
}

impl Images {
    pub fn with(mut self, n: u64, picture: Picture) -> Images {
        self.pictures.insert(media(n), Arc::new(picture));
        self
    }
}

impl LayerImages for Images {
    fn video(&mut self, asset: MediaId, source_time: Tick) -> Option<Arc<Picture>> {
        self.video_requests.push((asset, source_time));
        self.pictures.get(&asset).cloned()
    }

    fn still(&mut self, asset: MediaId) -> Option<Arc<Picture>> {
        self.pictures.get(&asset).cloned()
    }

    fn text(&mut self, layer: &LayerPlan) -> Option<TextLayer> {
        self.texts.get(&layer.clip).cloned()
    }
}
