//! E-2 / 32 §2 — analysis-as-node foundation.
//!
//! Analysis nodes emit **typed metadata**, not pixels. Results are cached by
//! content hash so re-analysis after an undo is free. Consumers (scopes,
//! loudness-on-export, scene detect, …) read the cached result rather than
//! re-running the analysis on every frame.
//!
//! Pull-based contract (26 E-2): given a timeline position and an input frame,
//! synchronously return the analysis — stateless, order-independent, seek-correct.

use std::collections::HashMap;

use crate::graph::ir::ContentHash;
use crate::graph::ops::Image;

/// Typed analysis payload (32 §2). Extend with Motion/SceneCuts/Transform as
/// consumers land — never a string property bag.
#[derive(Clone, Debug, PartialEq)]
pub enum AnalysisResult {
    /// 256-bin Rec.709 luma histogram (counts sum to sampled pixels).
    Histogram { bins: [u32; 256], samples: u32 },
    /// Per-channel mean / peak (linear premultiplied space).
    Levels {
        mean: [f32; 4],
        peak: [f32; 4],
        samples: u32,
    },
}

/// Context for analysis (tick + optional canvas hint). Reserved for multi-frame
/// windowed analysis.
#[derive(Copy, Clone, Debug, Default)]
pub struct AnalysisCtx {
    pub at: photonic_core::timeline::Tick,
}

/// In-memory analysis cache keyed by content hash of (op identity + input hash).
#[derive(Default)]
pub struct AnalysisCache {
    map: HashMap<u128, AnalysisResult>,
}

impl AnalysisCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: ContentHash) -> Option<&AnalysisResult> {
        self.map.get(&key.0)
    }

    pub fn insert(&mut self, key: ContentHash, value: AnalysisResult) {
        self.map.insert(key.0, value);
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Build a cache key from an analysis kind tag and the input frame content hash.
pub fn analysis_key(kind_tag: u8, input_hash: ContentHash) -> ContentHash {
    ContentHash((kind_tag as u128) << 120 | (input_hash.0 & ((1u128 << 120) - 1)))
}

/// 256-bin Rec.709 luma histogram of `img` (every pixel, premultiplied → approx
/// via max-alpha guard). Pure function of the pixels.
pub fn analyze_histogram(img: &Image) -> AnalysisResult {
    let mut bins = [0u32; 256];
    let mut samples = 0u32;
    for px in &img.pixels {
        let a = px[3].max(1e-4);
        let r = (px[0] / a).clamp(0.0, 1.0);
        let g = (px[1] / a).clamp(0.0, 1.0);
        let b = (px[2] / a).clamp(0.0, 1.0);
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let bin = (y * 255.0).round().clamp(0.0, 255.0) as usize;
        bins[bin] += 1;
        samples += 1;
    }
    AnalysisResult::Histogram { bins, samples }
}

/// Per-channel mean and peak of premultiplied linear pixels.
pub fn analyze_levels(img: &Image) -> AnalysisResult {
    let mut sum = [0.0f64; 4];
    let mut peak = [0.0f32; 4];
    let n = img.pixels.len() as f64;
    for px in &img.pixels {
        for c in 0..4 {
            sum[c] += px[c] as f64;
            peak[c] = peak[c].max(px[c]);
        }
    }
    let samples = img.pixels.len() as u32;
    let mean = if n > 0.0 {
        [
            (sum[0] / n) as f32,
            (sum[1] / n) as f32,
            (sum[2] / n) as f32,
            (sum[3] / n) as f32,
        ]
    } else {
        [0.0; 4]
    };
    AnalysisResult::Levels {
        mean,
        peak,
        samples,
    }
}

/// Cached histogram: returns prior result when `key` hits.
pub fn histogram_cached(
    cache: &mut AnalysisCache,
    key: ContentHash,
    img: &Image,
) -> AnalysisResult {
    if let Some(hit) = cache.get(key) {
        return hit.clone();
    }
    let result = analyze_histogram(img);
    cache.insert(key, result.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::ir::LinearColor;
    use crate::graph::ops::Image;

    #[test]
    fn histogram_of_uniform_gray_peaks_one_bin() {
        let img = Image::filled(
            4,
            4,
            LinearColor {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 1.0,
            },
        );
        let AnalysisResult::Histogram { bins, samples } = analyze_histogram(&img) else {
            panic!("expected histogram");
        };
        assert_eq!(samples, 16);
        let non_zero: Vec<_> = bins
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .collect();
        assert_eq!(non_zero.len(), 1);
        assert_eq!(*non_zero[0].1, 16);
    }

    #[test]
    fn cache_hits_on_same_key() {
        let mut cache = AnalysisCache::new();
        let img = Image::filled(
            2,
            2,
            LinearColor {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        );
        let key = analysis_key(1, ContentHash(0xABC));
        let a = histogram_cached(&mut cache, key, &img);
        let b = histogram_cached(&mut cache, key, &img);
        assert_eq!(a, b);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn levels_mean_of_solid() {
        let img = Image::filled(
            2,
            2,
            LinearColor {
                r: 0.25,
                g: 0.5,
                b: 0.75,
                a: 1.0,
            },
        );
        let AnalysisResult::Levels { mean, peak, samples } = analyze_levels(&img) else {
            panic!("expected levels");
        };
        assert_eq!(samples, 4);
        assert!((mean[0] - 0.25).abs() < 1e-5);
        assert!((mean[1] - 0.5).abs() < 1e-5);
        assert!((peak[2] - 0.75).abs() < 1e-5);
    }
}
