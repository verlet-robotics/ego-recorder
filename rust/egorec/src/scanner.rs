//! Streaming header-only scanner for .egorec v2 files.
//!
//! Reads only the file header, footer, index table, and frame block headers —
//! never touches compressed pixel data. Computes per-file idle baseline,
//! active/idle classification, and windowed activity statistics.
//!
//! Supports optional station-level baseline profiles for cross-recording
//! calibration and per-window depth analysis with RGB-depth fusion.

use crate::format::*;
use bitvec::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, BufReader, Seek, SeekFrom};

/// Configuration for the scanning pass.
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Window length in frames (default: 60 = 2s at 30fps).
    pub window_length_frames: u64,
    /// Window stride in frames (default: 15 = 0.5s).
    pub window_stride_frames: u64,
    /// Fraction of lowest P-frame sizes used for idle baseline (default: 0.25).
    pub idle_percentile: f32,
    /// RGB activity threshold = median + k * MAD (default: 3.0).
    pub activity_k: f32,
    /// Minimum consecutive active frames to count as a burst (default: 5).
    pub burst_min_frames: u32,
    /// Depth activity onset threshold multiplier (default: 3.0).
    pub depth_activity_k_onset: f32,
    /// Depth activity offset threshold multiplier for hysteresis (default: 2.0).
    pub depth_activity_k_offset: f32,
    /// Per-window Pearson r above this = ego/camera motion (default: 0.7).
    pub ego_motion_corr_threshold: f32,
    /// MAD → std-dev consistency constant for depth (default: 1.4826).
    pub mad_consistency: f32,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            window_length_frames: 60,
            window_stride_frames: 15,
            idle_percentile: 0.25,
            activity_k: 3.0,
            burst_min_frames: 5,
            depth_activity_k_onset: 3.0,
            depth_activity_k_offset: 2.0,
            ego_motion_corr_threshold: 0.7,
            mad_consistency: 1.4826,
        }
    }
}

/// Station-level idle reference baseline, aggregated across multiple recordings.
///
/// Built from the bottom percentile of P-frame sizes across all recordings at a station.
/// More robust than per-episode calibration when >75% of a recording is active.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationProfile {
    pub rgb_median: f32,
    pub rgb_mad: f32,
    pub depth_median: f32,
    pub depth_mad: f32,
    /// Total P-frames used to build this profile.
    pub frame_count: u64,
    /// Number of recordings that contributed.
    pub recording_count: u32,
}

impl StationProfile {
    /// Build a station profile by aggregating idle baselines across multiple scan summaries.
    /// Takes the bottom `idle_percentile` of P-frame sizes across ALL recordings combined.
    pub fn merge(summaries: &[ScanSummary], idle_percentile: f32) -> Self {
        let mut all_rgb: Vec<f32> = Vec::new();
        let mut all_depth: Vec<f32> = Vec::new();

        for summary in summaries {
            for fi in &summary.frame_infos {
                if !fi.is_expected_keyframe {
                    all_rgb.push(fi.rgb_compressed_size as f32);
                    all_depth.push(fi.depth_compressed_size as f32);
                }
            }
        }

        let frame_count = all_rgb.len() as u64;
        let recording_count = summaries.len() as u32;

        let rgb_baseline = compute_baseline(&mut all_rgb, idle_percentile, 3.0, 1.0);
        let depth_baseline = compute_baseline(&mut all_depth, idle_percentile, 3.0, 1.4826);

        Self {
            rgb_median: rgb_baseline.median,
            rgb_mad: rgb_baseline.mad,
            depth_median: depth_baseline.median,
            depth_mad: depth_baseline.mad,
            frame_count,
            recording_count,
        }
    }
}

/// Welford's online running statistics.
#[derive(Debug, Clone, Serialize)]
pub struct RunningStats {
    pub count: u64,
    pub mean: f64,
    pub m2: f64,
    pub min: f64,
    pub max: f64,
}

impl RunningStats {
    pub fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }

    pub fn push(&mut self, x: f64) {
        self.count += 1;
        let delta = x - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;
        if x < self.min {
            self.min = x;
        }
        if x > self.max {
            self.max = x;
        }
    }

    pub fn variance(&self) -> f64 {
        if self.count < 2 {
            0.0
        } else {
            self.m2 / (self.count - 1) as f64
        }
    }

    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }
}

/// Idle baseline computed from lowest percentile of P-frame sizes.
#[derive(Debug, Clone, Serialize)]
pub struct IdleBaseline {
    pub median: f32,
    pub mad: f32,
    pub threshold: f32,
}

/// Aggregated statistics for one overlapping window.
#[derive(Debug, Clone, Serialize)]
pub struct WindowStats {
    pub start_frame: u64,
    pub end_frame: u64,
    /// Fused active frame fraction (RGB+depth).
    pub active_frame_fraction: f32,
    pub mean_p_excess: f32,
    pub max_p_excess: f32,
    pub p95_p50_ratio: f32,
    pub burst_count: u32,
    /// Depth coefficient of variation (std/mean) within this window.
    pub depth_cv: f32,
    /// Fraction of depth-active P-frames in this window.
    pub depth_active_fraction: f32,
    /// Fraction of RGB-active P-frames in this window.
    pub rgb_active_fraction: f32,
    /// Pearson r(rgb_sizes, depth_sizes) within this window.
    pub rgb_depth_correlation: f32,
    /// Whether this window shows ego/camera motion (correlation > threshold).
    pub is_ego_motion: bool,
}

/// Lightweight per-frame info retained from header scan.
#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    pub frame_number: u64,
    pub timestamp_us: u64,
    pub rgb_compressed_size: u32,
    pub depth_compressed_size: u32,
    pub file_offset: u64,
    pub block_size: u32,
    pub is_expected_keyframe: bool,
}

/// Complete scan result for one .egorec file.
#[derive(Debug, Clone, Serialize)]
pub struct ScanSummary {
    pub total_frames: u64,
    pub duration_us: u64,
    pub start_timestamp_us: u64,
    pub rgb_p_stats: RunningStats,
    pub depth_p_stats: RunningStats,
    /// RGB idle baseline.
    pub idle_baseline: IdleBaseline,
    /// Depth idle baseline.
    pub depth_idle_baseline: IdleBaseline,
    pub windows: Vec<WindowStats>,
    /// Fused active mask (RGB+depth). Consumed by compute_segments().
    #[serde(skip)]
    pub active_mask: BitVec,
    /// RGB-only active mask (preserved for calibration/debug).
    #[serde(skip)]
    pub rgb_active_mask: BitVec,
    /// Depth-only active mask (with hysteresis).
    #[serde(skip)]
    pub depth_active_mask: BitVec,
    pub keyframe_positions: Vec<u64>,
    /// Per-frame info for splice operations.
    #[serde(skip)]
    pub frame_infos: Vec<FrameInfo>,
    /// Whether a station profile was used for baseline calibration.
    pub used_profile: bool,
}

/// Validation result from structural integrity checks.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub total_frames: u64,
    pub duration_us: u64,
    pub has_footer: bool,
    pub has_index: bool,
    pub index_entries: u32,
}

/// Scanner error types.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid .egorec file: {0}")]
    Format(String),
}

/// Compute idle baseline from the lowest percentile of frame sizes.
///
/// - `sizes`: mutable slice of P-frame sizes (will be sorted in place)
/// - `idle_percentile`: fraction of lowest sizes to use (e.g. 0.25)
/// - `k`: threshold multiplier
/// - `mad_scale`: MAD → σ conversion (1.0 for raw MAD, 1.4826 for normal-equivalent σ)
///
/// Threshold = median + k × MAD × mad_scale
fn compute_baseline(sizes: &mut [f32], idle_percentile: f32, k: f32, mad_scale: f32) -> IdleBaseline {
    if sizes.is_empty() {
        return IdleBaseline {
            median: 0.0,
            mad: 0.0,
            threshold: 0.0,
        };
    }

    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let cutoff = ((sizes.len() as f32 * idle_percentile).ceil() as usize).max(1);
    let idle_slice = &sizes[..cutoff];

    let median = if idle_slice.len() % 2 == 0 {
        (idle_slice[idle_slice.len() / 2 - 1] + idle_slice[idle_slice.len() / 2]) / 2.0
    } else {
        idle_slice[idle_slice.len() / 2]
    };

    let mut deviations: Vec<f32> = idle_slice.iter().map(|&x| (x - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = if deviations.len() % 2 == 0 && deviations.len() >= 2 {
        (deviations[deviations.len() / 2 - 1] + deviations[deviations.len() / 2]) / 2.0
    } else if !deviations.is_empty() {
        deviations[deviations.len() / 2]
    } else {
        0.0
    };

    let threshold = median + k * mad * mad_scale;

    IdleBaseline {
        median,
        mad,
        threshold,
    }
}

/// Compute Pearson correlation coefficient between two slices.
fn pearson_r(xs: &[f32], ys: &[f32]) -> f32 {
    let n = xs.len().min(ys.len());
    if n < 2 {
        return 0.0;
    }

    let mean_x = xs[..n].iter().map(|&x| x as f64).sum::<f64>() / n as f64;
    let mean_y = ys[..n].iter().map(|&y| y as f64).sum::<f64>() / n as f64;

    let mut cov = 0.0f64;
    let mut var_x = 0.0f64;
    let mut var_y = 0.0f64;
    for i in 0..n {
        let dx = xs[i] as f64 - mean_x;
        let dy = ys[i] as f64 - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }

    let denom = (var_x * var_y).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        (cov / denom) as f32
    }
}

pub struct EgorecScanner;

impl EgorecScanner {
    /// Validate structural integrity of an .egorec file.
    pub fn validate(path: &std::path::Path) -> Result<ValidationResult, ScanError> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let mut file = BufReader::new(File::open(path)?);

        // Check file header magic
        let header = match FileHeader::read_from(&mut file) {
            Ok(h) => h,
            Err(e) => {
                return Ok(ValidationResult {
                    valid: false,
                    errors: vec![format!("cannot read file header: {e}")],
                    warnings: vec![],
                    total_frames: 0,
                    duration_us: 0,
                    has_footer: false,
                    has_index: false,
                    index_entries: 0,
                });
            }
        };

        if header.magic[..6] != FILE_MAGIC[..6] {
            errors.push("bad magic bytes".into());
        }
        if header.magic[6] != 0x02 {
            errors.push(format!("unsupported version: {}", header.magic[6]));
        }

        // Check footer
        let file_len = file.seek(SeekFrom::End(0))?;
        let has_footer;
        let mut has_index = false;
        let mut index_entries = 0u32;
        let mut total_frames = 0u64;
        let mut duration_us = 0u64;

        if file_len < (FILE_HEADER_SIZE + FileFooter::SIZE) as u64 {
            errors.push("file too small for header + footer".into());
            has_footer = false;
        } else {
            file.seek(SeekFrom::End(-(FileFooter::SIZE as i64)))?;
            match FileFooter::read_from(&mut file) {
                Ok(footer) if footer.footer_magic == FOOTER_MAGIC => {
                    has_footer = true;
                    total_frames = footer.total_frames;
                    duration_us = footer.total_duration_us;
                    index_entries = footer.index_entry_count;

                    // Validate index magic
                    if footer.index_magic != INDEX_MAGIC {
                        errors.push("bad index magic in footer".into());
                    }

                    // Validate index offset bounds
                    if footer.index_offset < FILE_HEADER_SIZE as u64 {
                        errors.push(format!(
                            "index_offset {} before end of file header",
                            footer.index_offset
                        ));
                    }

                    let expected_index_size =
                        footer.index_entry_count as u64 * IndexEntry::SIZE as u64;
                    let footer_start = file_len - FileFooter::SIZE as u64;
                    if footer.index_offset + expected_index_size > footer_start {
                        errors.push("index table overlaps footer".into());
                    }

                    // Read and validate index entries
                    if footer.index_entry_count > 0 && errors.is_empty() {
                        has_index = true;
                        file.seek(SeekFrom::Start(footer.index_offset))?;
                        let mut prev_ts = 0u64;
                        let mut prev_offset = 0u64;
                        for i in 0..footer.index_entry_count {
                            match IndexEntry::read_from(&mut file) {
                                Ok(entry) => {
                                    if i > 0 {
                                        if entry.file_offset <= prev_offset {
                                            errors.push(format!(
                                                "index entry {i}: non-increasing offset"
                                            ));
                                        }
                                        if entry.timestamp_us < prev_ts {
                                            warnings.push(format!(
                                                "index entry {i}: non-monotonic timestamp"
                                            ));
                                        }
                                    }
                                    if entry.file_offset < FILE_HEADER_SIZE as u64 {
                                        errors.push(format!(
                                            "index entry {i}: offset before file header"
                                        ));
                                    }
                                    prev_ts = entry.timestamp_us;
                                    prev_offset = entry.file_offset;
                                }
                                Err(e) => {
                                    errors.push(format!("cannot read index entry {i}: {e}"));
                                    break;
                                }
                            }
                        }

                        // Validate frame count matches index
                        if footer.total_frames != footer.index_entry_count as u64 {
                            warnings.push(format!(
                                "frame count mismatch: footer says {} frames but {} index entries",
                                footer.total_frames, footer.index_entry_count
                            ));
                        }
                    }

                    // Spot-check first frame block
                    file.seek(SeekFrom::Start(FILE_HEADER_SIZE as u64))?;
                    if let Ok(fbh) = FrameBlockHeader::read_from(&mut file) {
                        if fbh.magic != FRAME_MAGIC {
                            errors.push("first frame block has bad magic".into());
                        }
                    }
                }
                _ => {
                    has_footer = false;
                    warnings.push("no valid footer — file may be truncated".into());
                }
            }
        }

        Ok(ValidationResult {
            valid: errors.is_empty(),
            errors,
            warnings,
            total_frames,
            duration_us,
            has_footer,
            has_index,
            index_entries,
        })
    }

    /// Streaming two-pass scan with per-episode calibration (backward compatible).
    pub fn scan(path: &std::path::Path, config: &ScanConfig) -> Result<ScanSummary, ScanError> {
        Self::scan_with_profile(path, config, None)
    }

    /// Streaming scan with optional station-level baseline profile.
    ///
    /// When a profile is provided, baselines are derived from the station-level
    /// reference instead of per-episode calibration. This is more robust when
    /// >75% of a recording is active (baseline contamination).
    pub fn scan_with_profile(
        path: &std::path::Path,
        config: &ScanConfig,
        profile: Option<&StationProfile>,
    ) -> Result<ScanSummary, ScanError> {
        let mut file = BufReader::new(File::open(path)?);

        // Read header
        let header = FileHeader::read_from(&mut file)
            .map_err(|e| ScanError::Format(format!("cannot read header: {e}")))?;
        if header.magic[..6] != FILE_MAGIC[..6] || header.magic[6] != 0x02 {
            return Err(ScanError::Format("bad magic or version".into()));
        }

        // Read footer
        file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::End(-(FileFooter::SIZE as i64)))?;
        let footer = FileFooter::read_from(&mut file)
            .map_err(|e| ScanError::Format(format!("cannot read footer: {e}")))?;
        if footer.footer_magic != FOOTER_MAGIC {
            return Err(ScanError::Format("missing footer magic".into()));
        }

        let total_frames = footer.total_frames;
        let duration_us = footer.total_duration_us;

        // Read index table
        let mut index = Vec::with_capacity(footer.index_entry_count as usize);
        if footer.index_entry_count > 0 {
            file.seek(SeekFrom::Start(footer.index_offset))?;
            for _ in 0..footer.index_entry_count {
                index.push(IndexEntry::read_from(&mut file)?);
            }
        }

        // Scan frame headers
        let frame_infos = if index.is_empty() {
            Self::scan_sequential(&mut file, total_frames)?
        } else {
            Self::scan_indexed(&mut file, &index)?
        };

        // Collect P-frame sizes and running stats
        let mut p_frame_rgb_sizes: Vec<f32> = Vec::new();
        let mut p_frame_depth_sizes: Vec<f32> = Vec::new();
        let mut rgb_p_stats = RunningStats::new();
        let mut depth_p_stats = RunningStats::new();
        let mut keyframe_positions = Vec::new();

        for fi in &frame_infos {
            if fi.is_expected_keyframe {
                keyframe_positions.push(fi.frame_number);
            } else {
                let rgb_size = fi.rgb_compressed_size as f64;
                let depth_size = fi.depth_compressed_size as f64;
                rgb_p_stats.push(rgb_size);
                depth_p_stats.push(depth_size);
                p_frame_rgb_sizes.push(fi.rgb_compressed_size as f32);
                p_frame_depth_sizes.push(fi.depth_compressed_size as f32);
            }
        }

        let has_depth = depth_p_stats.mean > 1.0;
        let used_profile = profile.is_some();

        // Compute baselines (from profile or per-episode)
        let (rgb_baseline, depth_baseline) = if let Some(prof) = profile {
            let rgb = IdleBaseline {
                median: prof.rgb_median,
                mad: prof.rgb_mad,
                threshold: prof.rgb_median + config.activity_k * prof.rgb_mad,
            };
            let depth = IdleBaseline {
                median: prof.depth_median,
                mad: prof.depth_mad,
                threshold: prof.depth_median
                    + config.depth_activity_k_onset * prof.depth_mad * config.mad_consistency,
            };
            (rgb, depth)
        } else {
            let rgb = compute_baseline(
                &mut p_frame_rgb_sizes,
                config.idle_percentile,
                config.activity_k,
                1.0,
            );
            let depth = if has_depth {
                compute_baseline(
                    &mut p_frame_depth_sizes,
                    config.idle_percentile,
                    config.depth_activity_k_onset,
                    config.mad_consistency,
                )
            } else {
                IdleBaseline {
                    median: 0.0,
                    mad: 0.0,
                    threshold: 0.0,
                }
            };
            (rgb, depth)
        };

        // Build RGB active mask
        let mut rgb_active_mask = bitvec![0; frame_infos.len()];
        for (i, fi) in frame_infos.iter().enumerate() {
            if !fi.is_expected_keyframe && fi.rgb_compressed_size as f32 > rgb_baseline.threshold {
                rgb_active_mask.set(i, true);
            }
        }

        // Build depth active mask with hysteresis (onset/offset thresholds)
        let mut depth_active_mask = bitvec![0; frame_infos.len()];
        if has_depth {
            let onset = depth_baseline.median
                + config.depth_activity_k_onset * depth_baseline.mad * config.mad_consistency;
            let offset = depth_baseline.median
                + config.depth_activity_k_offset * depth_baseline.mad * config.mad_consistency;

            let mut in_active = false;
            for (i, fi) in frame_infos.iter().enumerate() {
                if fi.is_expected_keyframe {
                    // Carry forward previous state for keyframes (always large, don't trigger/cancel)
                    if in_active {
                        depth_active_mask.set(i, true);
                    }
                } else {
                    let size = fi.depth_compressed_size as f32;
                    if in_active {
                        if size < offset {
                            in_active = false;
                        } else {
                            depth_active_mask.set(i, true);
                        }
                    } else if size > onset {
                        in_active = true;
                        depth_active_mask.set(i, true);
                    }
                }
            }
        }

        // Compute per-window stats (with depth features and correlation)
        let windows = Self::compute_windows(
            &frame_infos,
            &rgb_active_mask,
            &depth_active_mask,
            &rgb_baseline,
            config,
            has_depth,
        );

        // Build fused active mask:
        //   fused[i] = depth_active[i] OR (rgb_active[i] AND NOT window_is_ego_motion(i))
        let mut fused_mask = bitvec![0; frame_infos.len()];
        let wstride = config.window_stride_frames as usize;
        let wlen = config.window_length_frames as usize;

        for i in 0..frame_infos.len() {
            if depth_active_mask[i] {
                // Depth active = real 3D change → always trust
                fused_mask.set(i, true);
            } else if rgb_active_mask[i] {
                // RGB-only active: check if the enclosing window is ego motion
                let is_ego = if !windows.is_empty() && wstride > 0 {
                    let window_idx = i.saturating_sub(wlen / 2) / wstride;
                    let window_idx = window_idx.min(windows.len() - 1);
                    windows[window_idx].is_ego_motion
                } else {
                    false
                };
                if !is_ego {
                    // RGB-only active in non-ego-motion window → subtle manipulation
                    fused_mask.set(i, true);
                }
                // RGB-only active in ego-motion window → camera panning, demote to idle
            }
        }

        Ok(ScanSummary {
            total_frames,
            duration_us,
            start_timestamp_us: header.start_timestamp_us,
            rgb_p_stats,
            depth_p_stats,
            idle_baseline: rgb_baseline,
            depth_idle_baseline: depth_baseline,
            windows,
            active_mask: fused_mask,
            rgb_active_mask,
            depth_active_mask,
            keyframe_positions,
            frame_infos,
            used_profile,
        })
    }

    fn scan_indexed(
        file: &mut BufReader<File>,
        index: &[IndexEntry],
    ) -> Result<Vec<FrameInfo>, ScanError> {
        let mut infos = Vec::with_capacity(index.len());
        for (i, entry) in index.iter().enumerate() {
            file.seek(SeekFrom::Start(entry.file_offset))?;
            let fbh = FrameBlockHeader::read_from(file)?;
            if fbh.magic != FRAME_MAGIC {
                return Err(ScanError::Format(format!("bad frame magic at index {i}")));
            }
            infos.push(FrameInfo {
                frame_number: fbh.frame_number,
                timestamp_us: fbh.timestamp_us,
                rgb_compressed_size: fbh.rgb_compressed_size,
                depth_compressed_size: fbh.depth_compressed_size,
                file_offset: entry.file_offset,
                block_size: fbh.block_size,
                is_expected_keyframe: fbh.frame_number % 30 == 0,
            });
        }
        Ok(infos)
    }

    fn scan_sequential(
        file: &mut BufReader<File>,
        total_frames: u64,
    ) -> Result<Vec<FrameInfo>, ScanError> {
        file.seek(SeekFrom::Start(FILE_HEADER_SIZE as u64))?;
        let mut infos = Vec::with_capacity(total_frames as usize);
        for _ in 0..total_frames {
            let offset = file.seek(SeekFrom::Current(0))?;
            let fbh = FrameBlockHeader::read_from(file)?;
            if fbh.magic != FRAME_MAGIC {
                return Err(ScanError::Format("bad frame magic in sequential scan".into()));
            }
            infos.push(FrameInfo {
                frame_number: fbh.frame_number,
                timestamp_us: fbh.timestamp_us,
                rgb_compressed_size: fbh.rgb_compressed_size,
                depth_compressed_size: fbh.depth_compressed_size,
                file_offset: offset,
                block_size: fbh.block_size,
                is_expected_keyframe: fbh.frame_number % 30 == 0,
            });
            // Skip past the frame data to the next frame block
            let data_size = fbh.block_size as u64 - FrameBlockHeader::SIZE as u64;
            file.seek(SeekFrom::Current(data_size as i64))?;
        }
        Ok(infos)
    }

    fn compute_windows(
        frame_infos: &[FrameInfo],
        rgb_active_mask: &BitVec,
        depth_active_mask: &BitVec,
        rgb_baseline: &IdleBaseline,
        config: &ScanConfig,
        has_depth: bool,
    ) -> Vec<WindowStats> {
        let n = frame_infos.len() as u64;
        if n == 0 {
            return vec![];
        }

        let mut windows = Vec::new();
        let mut start = 0u64;
        while start < n {
            let end = (start + config.window_length_frames).min(n);
            let window_slice = &frame_infos[start as usize..end as usize];
            let rgb_mask_slice = &rgb_active_mask[start as usize..end as usize];
            let depth_mask_slice = &depth_active_mask[start as usize..end as usize];

            let mut rgb_active_p = 0u32;
            let mut depth_active_p = 0u32;
            let mut total_p = 0u32;
            let mut rgb_p_sizes: Vec<f32> = Vec::new();
            let mut depth_p_sizes: Vec<f32> = Vec::new();
            let mut max_excess: f32 = 0.0;
            let mut sum_excess: f64 = 0.0;

            for (j, fi) in window_slice.iter().enumerate() {
                if !fi.is_expected_keyframe {
                    total_p += 1;
                    let rgb_size = fi.rgb_compressed_size as f32;
                    let depth_size = fi.depth_compressed_size as f32;
                    rgb_p_sizes.push(rgb_size);
                    depth_p_sizes.push(depth_size);

                    if rgb_mask_slice[j] {
                        rgb_active_p += 1;
                    }
                    if depth_mask_slice[j] {
                        depth_active_p += 1;
                    }

                    if rgb_baseline.median > 0.0 {
                        let excess = rgb_size / rgb_baseline.median;
                        sum_excess += excess as f64;
                        if excess > max_excess {
                            max_excess = excess;
                        }
                    }
                }
            }

            let rgb_active_frac = if total_p > 0 {
                rgb_active_p as f32 / total_p as f32
            } else {
                0.0
            };
            let depth_active_frac = if total_p > 0 {
                depth_active_p as f32 / total_p as f32
            } else {
                0.0
            };

            let mean_excess = if total_p > 0 && rgb_baseline.median > 0.0 {
                (sum_excess / total_p as f64) as f32
            } else {
                0.0
            };

            // p95/p50 ratio (RGB)
            let mut sorted_rgb = rgb_p_sizes.clone();
            sorted_rgb.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let p95_p50 = if sorted_rgb.len() >= 2 {
                let p50_idx = sorted_rgb.len() / 2;
                let p95_idx = (sorted_rgb.len() as f32 * 0.95) as usize;
                let p95_idx = p95_idx.min(sorted_rgb.len() - 1);
                if sorted_rgb[p50_idx] > 0.0 {
                    sorted_rgb[p95_idx] / sorted_rgb[p50_idx]
                } else {
                    1.0
                }
            } else {
                1.0
            };

            // Depth CV within window
            let depth_cv = if has_depth && !depth_p_sizes.is_empty() {
                let mean_d =
                    depth_p_sizes.iter().map(|&x| x as f64).sum::<f64>() / depth_p_sizes.len() as f64;
                if mean_d > 0.0 {
                    let variance = depth_p_sizes
                        .iter()
                        .map(|&x| {
                            let d = x as f64 - mean_d;
                            d * d
                        })
                        .sum::<f64>()
                        / depth_p_sizes.len().max(1) as f64;
                    (variance.sqrt() / mean_d) as f32
                } else {
                    0.0
                }
            } else {
                0.0
            };

            // RGB-depth correlation within window (need >= 10 P-frames)
            let correlation = if has_depth && rgb_p_sizes.len() >= 10 {
                pearson_r(&rgb_p_sizes, &depth_p_sizes)
            } else {
                0.0
            };

            let is_ego_motion = has_depth && correlation > config.ego_motion_corr_threshold;

            // Fused active fraction: depth OR (rgb AND NOT ego_motion)
            let mut fused_active_p = 0u32;
            for (j, fi) in window_slice.iter().enumerate() {
                if !fi.is_expected_keyframe {
                    let fused = depth_mask_slice[j] || (rgb_mask_slice[j] && !is_ego_motion);
                    if fused {
                        fused_active_p += 1;
                    }
                }
            }
            let fused_active_frac = if total_p > 0 {
                fused_active_p as f32 / total_p as f32
            } else {
                0.0
            };

            // Count bursts of consecutive fused-active frames
            let mut burst_count = 0u32;
            let mut run = 0u32;
            for (j, _fi) in window_slice.iter().enumerate() {
                let fused = depth_mask_slice[j] || (rgb_mask_slice[j] && !is_ego_motion);
                if fused {
                    run += 1;
                } else {
                    if run >= config.burst_min_frames {
                        burst_count += 1;
                    }
                    run = 0;
                }
            }
            if run >= config.burst_min_frames {
                burst_count += 1;
            }

            windows.push(WindowStats {
                start_frame: start,
                end_frame: end,
                active_frame_fraction: fused_active_frac,
                mean_p_excess: mean_excess,
                max_p_excess: max_excess,
                p95_p50_ratio: p95_p50,
                burst_count,
                depth_cv,
                depth_active_fraction: depth_active_frac,
                rgb_active_fraction: rgb_active_frac,
                rgb_depth_correlation: correlation,
                is_ego_motion,
            });

            start += config.window_stride_frames;
        }

        windows
    }

    pub(crate) fn count_bursts(mask: &bitvec::slice::BitSlice, min_len: u32) -> u32 {
        let mut bursts = 0u32;
        let mut run = 0u32;
        for bit in mask.iter() {
            if *bit {
                run += 1;
            } else {
                if run >= min_len {
                    bursts += 1;
                }
                run = 0;
            }
        }
        if run >= min_len {
            bursts += 1;
        }
        bursts
    }
}

/// Episode-level features computed from ScanSummary.
#[derive(Debug, Clone, Serialize)]
pub struct EpisodeFeatures {
    /// Fraction of P-frames classified as active (fused RGB+depth).
    pub active_frame_fraction: f32,
    pub burst_count: u32,
    pub p95_p50_ratio: f32,
    pub first_active_window: Option<usize>,
    pub last_active_window: Option<usize>,
    pub longest_idle_prefix: usize,
    pub longest_idle_suffix: usize,
    pub final_third_activity: f32,
    pub active_window_fraction: f32,
    /// Depth coefficient of variation (std/mean of depth P-frame compressed sizes).
    /// High values (>0.15) indicate real 3D scene changes (hands, objects moving).
    /// Low values (<0.10) indicate a static scene with only camera/lighting noise.
    pub depth_cv: f32,
    /// Fraction of depth-active P-frames (from depth hysteresis mask).
    pub depth_active_frame_fraction: f32,
    /// Mean depth CV across all windows.
    pub window_depth_cv_mean: f32,
    /// Maximum depth CV across all windows (localized manipulation spikes).
    pub window_depth_cv_max: f32,
    /// Fraction of windows classified as ego/camera motion.
    pub ego_motion_window_fraction: f32,
}

impl EpisodeFeatures {
    pub fn from_summary(summary: &ScanSummary, config: &ScanConfig) -> Self {
        let n = summary.active_mask.len();

        // active_frame_fraction: fraction of P-frames above threshold (fused)
        let total_p = summary
            .frame_infos
            .iter()
            .filter(|f| !f.is_expected_keyframe)
            .count();
        let active_p = summary
            .frame_infos
            .iter()
            .enumerate()
            .filter(|(i, f)| !f.is_expected_keyframe && summary.active_mask[*i])
            .count();
        let active_frame_fraction = if total_p > 0 {
            active_p as f32 / total_p as f32
        } else {
            0.0
        };

        // depth_active_frame_fraction from depth mask
        let depth_active_p = summary
            .frame_infos
            .iter()
            .enumerate()
            .filter(|(i, f)| !f.is_expected_keyframe && summary.depth_active_mask[*i])
            .count();
        let depth_active_frame_fraction = if total_p > 0 {
            depth_active_p as f32 / total_p as f32
        } else {
            0.0
        };

        // burst_count over entire episode (fused mask)
        let burst_count =
            EgorecScanner::count_bursts(&summary.active_mask, config.burst_min_frames);

        // p95/p50 ratio of all P-frame sizes
        let mut all_p_sizes: Vec<f32> = summary
            .frame_infos
            .iter()
            .filter(|f| !f.is_expected_keyframe)
            .map(|f| f.rgb_compressed_size as f32)
            .collect();
        all_p_sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p95_p50_ratio = if all_p_sizes.len() >= 2 {
            let p50 = all_p_sizes[all_p_sizes.len() / 2];
            let p95 = all_p_sizes[(all_p_sizes.len() as f32 * 0.95) as usize];
            if p50 > 0.0 {
                p95 / p50
            } else {
                1.0
            }
        } else {
            1.0
        };

        // Window-level features
        let active_threshold = 0.2f32;
        let first_active_window = summary
            .windows
            .iter()
            .position(|w| w.active_frame_fraction > active_threshold);
        let last_active_window = summary
            .windows
            .iter()
            .rposition(|w| w.active_frame_fraction > active_threshold);

        let longest_idle_prefix = summary
            .windows
            .iter()
            .take_while(|w| w.active_frame_fraction <= active_threshold)
            .count();
        let longest_idle_suffix = summary
            .windows
            .iter()
            .rev()
            .take_while(|w| w.active_frame_fraction <= active_threshold)
            .count();

        let active_window_fraction = if !summary.windows.is_empty() {
            summary
                .windows
                .iter()
                .filter(|w| w.active_frame_fraction > 0.0)
                .count() as f32
                / summary.windows.len() as f32
        } else {
            0.0
        };

        // Final third activity (fused mask)
        let final_third_start = (n as f64 * 2.0 / 3.0) as usize;
        let final_third_total_p = summary.frame_infos[final_third_start..]
            .iter()
            .filter(|f| !f.is_expected_keyframe)
            .count();
        let final_third_active_p = summary.frame_infos[final_third_start..]
            .iter()
            .enumerate()
            .filter(|(i, f)| {
                !f.is_expected_keyframe && summary.active_mask[final_third_start + i]
            })
            .count();
        let final_third_activity = if final_third_total_p > 0 {
            final_third_active_p as f32 / final_third_total_p as f32
        } else {
            0.0
        };

        // Depth coefficient of variation (episode-level)
        let depth_cv = if summary.depth_p_stats.mean > 0.0 {
            (summary.depth_p_stats.std_dev() / summary.depth_p_stats.mean) as f32
        } else {
            0.0
        };

        // Window depth CV statistics
        let window_depth_cv_mean = if !summary.windows.is_empty() {
            summary.windows.iter().map(|w| w.depth_cv).sum::<f32>()
                / summary.windows.len() as f32
        } else {
            0.0
        };
        let window_depth_cv_max = summary
            .windows
            .iter()
            .map(|w| w.depth_cv)
            .fold(0.0f32, f32::max);

        // Ego motion window fraction
        let ego_motion_window_fraction = if !summary.windows.is_empty() {
            summary
                .windows
                .iter()
                .filter(|w| w.is_ego_motion)
                .count() as f32
                / summary.windows.len() as f32
        } else {
            0.0
        };

        Self {
            active_frame_fraction,
            burst_count,
            p95_p50_ratio,
            first_active_window,
            last_active_window,
            longest_idle_prefix,
            longest_idle_suffix,
            final_third_activity,
            active_window_fraction,
            depth_cv,
            depth_active_frame_fraction,
            window_depth_cv_mean,
            window_depth_cv_max,
            ego_motion_window_fraction,
        }
    }
}

/// Prune/keep verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Verdict {
    Keep,
    PruneConfident,
    PruneSuggested,
    Review,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Keep => write!(f, "KEEP"),
            Verdict::PruneConfident => write!(f, "PRUNE_CONFIDENT"),
            Verdict::PruneSuggested => write!(f, "PRUNE_SUGGESTED"),
            Verdict::Review => write!(f, "REVIEW"),
        }
    }
}

/// Full analysis result for one episode.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub filename: String,
    pub verdict: Verdict,
    pub activity_score: f32,
    pub reasons_keep: Vec<String>,
    pub reasons_prune: Vec<String>,
    pub features: EpisodeFeatures,
    pub total_frames: u64,
    pub duration_s: f64,
    pub idle_baseline: IdleBaseline,
    pub depth_idle_baseline: IdleBaseline,
    pub used_profile: bool,
}

impl AnalysisResult {
    pub fn compute(
        filename: &str,
        summary: &ScanSummary,
        features: &EpisodeFeatures,
    ) -> Self {
        let mut reasons_keep = Vec::new();
        let mut reasons_prune = Vec::new();

        let duration_s = summary.duration_us as f64 / 1e6;

        // --- Immediate PRUNE_CONFIDENT ---
        if summary.total_frames < 30 && duration_s < 1.0 {
            reasons_prune.push(format!(
                "accidental start: {} frames, {:.3}s",
                summary.total_frames, duration_s
            ));
            return Self {
                filename: filename.to_string(),
                verdict: Verdict::PruneConfident,
                activity_score: 0.0,
                reasons_keep,
                reasons_prune,
                features: features.clone(),
                total_frames: summary.total_frames,
                duration_s,
                idle_baseline: summary.idle_baseline.clone(),
                depth_idle_baseline: summary.depth_idle_baseline.clone(),
                used_profile: summary.used_profile,
            };
        }

        // --- Depth-based signals ---
        // depth_cv > 0.18 OR per-window depth spike → real 3D scene changes
        let has_depth_activity =
            features.depth_cv > 0.18 || features.window_depth_cv_max > 0.25;
        let has_no_depth_activity = features.depth_cv < 0.13 && features.window_depth_cv_max < 0.20;

        // --- Immediate KEEP signals ---

        // High depth CV = real manipulation (strongest signal)
        if features.depth_cv > 0.18 {
            reasons_keep.push(format!(
                "depth variation: CV={:.3} (3D scene changes detected)",
                features.depth_cv
            ));
        }

        // Per-window depth spike (localized manipulation even if episode-level CV is moderate)
        if features.window_depth_cv_max > 0.25 {
            reasons_keep.push(format!(
                "per-window depth spike: max_cv={:.3}",
                features.window_depth_cv_max
            ));
        }

        // Any window with >50% active AND depth confirms
        if let Some(w) = summary
            .windows
            .iter()
            .find(|w| w.active_frame_fraction > 0.5)
        {
            if has_depth_activity {
                reasons_keep.push(format!(
                    "sustained activity in window [{}-{}]: {:.0}% active",
                    w.start_frame,
                    w.end_frame,
                    w.active_frame_fraction * 100.0
                ));
            }
        }

        // Multiple bursts (only count as keep if depth also active)
        if features.burst_count >= 2 && has_depth_activity {
            reasons_keep.push(format!(
                "{} activity bursts detected",
                features.burst_count
            ));
        }

        // Late-episode activation (only count as keep if depth also active)
        if features.final_third_activity > 0.15 && has_depth_activity {
            reasons_keep.push(format!(
                "late episode activation: {:.0}% active in final third",
                features.final_third_activity * 100.0
            ));
        }

        // --- PRUNE_SUGGESTED signals ---
        if has_no_depth_activity {
            reasons_prune.push(format!(
                "static depth: CV={:.3} max_window={:.3} (no 3D scene changes)",
                features.depth_cv, features.window_depth_cv_max
            ));
        }
        if features.active_frame_fraction < 0.02 {
            reasons_prune.push(format!(
                "very low activity: {:.1}% of frames active (fused)",
                features.active_frame_fraction * 100.0
            ));
        }
        if features.burst_count == 0 {
            reasons_prune.push("no activity bursts".into());
        }
        if features.final_third_activity <= 0.0 {
            reasons_prune.push("no late-episode activation".into());
        }
        if features.active_window_fraction < 0.05 {
            reasons_prune.push(format!(
                "minimal window activity: {:.1}%",
                features.active_window_fraction * 100.0
            ));
        }
        if features.ego_motion_window_fraction > 0.8 {
            reasons_prune.push(format!(
                "mostly camera motion: {:.0}% ego-motion windows",
                features.ego_motion_window_fraction * 100.0
            ));
        }

        // Activity score (continuous)
        let activity_score = Self::compute_activity_score(features);

        // Decision: depth_cv is the primary discriminator
        let verdict = if !reasons_keep.is_empty() {
            Verdict::Keep
        } else if has_no_depth_activity {
            // Static depth scene — RGB "activity" is just camera noise
            Verdict::PruneSuggested
        } else if features.active_frame_fraction < 0.02
            && features.burst_count == 0
            && features.final_third_activity <= 0.0
        {
            Verdict::PruneSuggested
        } else {
            Verdict::Review
        };

        Self {
            filename: filename.to_string(),
            verdict,
            activity_score,
            reasons_keep,
            reasons_prune,
            features: features.clone(),
            total_frames: summary.total_frames,
            duration_s,
            idle_baseline: summary.idle_baseline.clone(),
            depth_idle_baseline: summary.depth_idle_baseline.clone(),
            used_profile: summary.used_profile,
        }
    }

    fn compute_activity_score(features: &EpisodeFeatures) -> f32 {
        // depth_cv is the primary signal — normalized to [0,1] with 0.20 as "full score"
        let depth_norm = (features.depth_cv / 0.20).clamp(0.0, 1.0);
        let aff = (features.active_frame_fraction * 5.0).min(1.0);
        let burst_norm = (features.burst_count as f32 / 5.0).min(1.0);
        let ratio_norm = ((features.p95_p50_ratio - 1.0) / 3.0).clamp(0.0, 1.0);
        let final_third = (features.final_third_activity * 3.0).min(1.0);

        (0.35 * depth_norm
            + 0.20 * aff
            + 0.15 * burst_norm
            + 0.10 * ratio_norm
            + 0.20 * final_third)
            .clamp(0.0, 1.0)
    }
}

/// Segment proposal for splice operations.
#[derive(Debug, Clone, Serialize)]
pub struct SegmentProposal {
    pub start_frame: usize,
    pub end_frame: usize,
    pub active_frames: usize,
    pub total_frames: usize,
}

impl ScanSummary {
    /// Compute splice segment proposals from the active mask.
    pub fn compute_segments(
        &self,
        min_gap_frames: usize,
        min_duration_frames: usize,
        pad_frames: usize,
    ) -> Vec<SegmentProposal> {
        let n = self.active_mask.len();
        if n == 0 {
            return vec![];
        }

        // Find runs of active frames
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut in_run = false;
        let mut run_start = 0;

        for i in 0..n {
            if self.active_mask[i] {
                if !in_run {
                    run_start = i;
                    in_run = true;
                }
            } else if in_run {
                ranges.push((run_start, i));
                in_run = false;
            }
        }
        if in_run {
            ranges.push((run_start, n));
        }

        if ranges.is_empty() {
            return vec![];
        }

        // Merge gaps smaller than min_gap_frames
        let mut merged: Vec<(usize, usize)> = vec![ranges[0]];
        for &(start, end) in &ranges[1..] {
            let last = merged.last_mut().unwrap();
            if start - last.1 < min_gap_frames {
                last.1 = end;
            } else {
                merged.push((start, end));
            }
        }

        // Remove short segments
        merged.retain(|&(s, e)| e - s >= min_duration_frames);

        // Pad and snap to keyframe grid (frame_number % 30 == 0)
        let mut segments = Vec::new();
        for (s, e) in merged {
            let padded_start = s.saturating_sub(pad_frames);
            let padded_end = (e + pad_frames).min(n);

            // Snap start down to keyframe grid
            let snapped_start = (padded_start / 30) * 30;
            // Snap end up to keyframe grid
            let snapped_end = ((padded_end + 29) / 30 * 30).min(n);

            let active_count = self.active_mask[snapped_start..snapped_end]
                .iter()
                .filter(|b| **b)
                .count();

            segments.push(SegmentProposal {
                start_frame: snapped_start,
                end_frame: snapped_end,
                active_frames: active_count,
                total_frames: snapped_end - snapped_start,
            });
        }

        // Merge any overlapping segments after snapping
        if segments.len() > 1 {
            let mut final_segments: Vec<SegmentProposal> = vec![segments[0].clone()];
            for seg in &segments[1..] {
                let last = final_segments.last_mut().unwrap();
                if seg.start_frame <= last.end_frame {
                    last.end_frame = seg.end_frame;
                    last.total_frames = last.end_frame - last.start_frame;
                    last.active_frames = self.active_mask[last.start_frame..last.end_frame]
                        .iter()
                        .filter(|b| **b)
                        .count();
                } else {
                    final_segments.push(seg.clone());
                }
            }
            final_segments
        } else {
            segments
        }
    }
}
