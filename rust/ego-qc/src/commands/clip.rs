use anyhow::{bail, Context, Result};
use egorec::{EgorecScanner, EgorecWriter, FileHeader, ScanConfig};
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

fn frame_range_from_timestamps(
    timestamps: &[u64],
    base_timestamp_us: u64,
    start_s: f64,
    end_s: f64,
) -> Result<(usize, usize)> {
    if timestamps.is_empty() {
        bail!("recording has no frames");
    }
    if !start_s.is_finite() || !end_s.is_finite() {
        bail!("start/end must be finite");
    }
    if start_s < 0.0 {
        bail!("start must be non-negative");
    }
    if end_s <= start_s {
        bail!("end must be greater than start");
    }

    let start_timestamp_us = base_timestamp_us.saturating_add((start_s * 1e6).round() as u64);
    let end_timestamp_us = base_timestamp_us.saturating_add((end_s * 1e6).round() as u64);

    let start_idx = timestamps
        .iter()
        .position(|timestamp_us| *timestamp_us >= start_timestamp_us)
        .context("requested start is beyond the end of the recording")?;
    let end_idx = timestamps
        .iter()
        .position(|timestamp_us| *timestamp_us > end_timestamp_us)
        .unwrap_or(timestamps.len());

    if end_idx <= start_idx {
        bail!("requested range resolves to an empty frame span");
    }

    Ok((start_idx, end_idx))
}

fn read_header(source_file: &mut File) -> Result<FileHeader> {
    let mut reader = BufReader::new(&mut *source_file);
    let header = FileHeader::read_from(&mut reader)?;
    source_file.seek(SeekFrom::Start(0))?;
    Ok(header)
}

pub fn run(input: &str, start_s: f64, end_s: f64, output: &str) -> Result<()> {
    let input_path = Path::new(input);
    let output_path = Path::new(output);

    let summary = EgorecScanner::scan(input_path, &ScanConfig::default())?;
    let timestamps: Vec<u64> = summary
        .frame_infos
        .iter()
        .map(|info| info.timestamp_us)
        .collect();
    let (start_idx, end_idx) =
        frame_range_from_timestamps(&timestamps, summary.start_timestamp_us, start_s, end_s)?;

    let frame_infos = &summary.frame_infos[start_idx..end_idx];
    let mut source_file = File::open(input_path)?;
    let mut header = read_header(&mut source_file)?;
    header.start_timestamp_us = frame_infos[0].timestamp_us;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut writer = EgorecWriter::create(output_path, &header)?;
    writer.copy_span(&mut source_file, frame_infos)?;
    writer.finalize()?;

    println!(
        "CLIP  {} -> {}  (frames {}-{}, {:.3}-{:.3}s)",
        input_path.display(),
        output_path.display(),
        start_idx,
        end_idx,
        start_s,
        end_s
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::frame_range_from_timestamps;

    #[test]
    fn frame_range_picks_expected_span() {
        let timestamps = [1_000_000, 1_100_000, 1_200_000, 1_300_000, 1_400_000];
        let (start, end) = frame_range_from_timestamps(&timestamps, 1_000_000, 0.05, 0.25).unwrap();
        assert_eq!((start, end), (1, 3));
    }

    #[test]
    fn frame_range_rejects_empty_window() {
        let timestamps = [1_000_000, 1_100_000];
        let err = frame_range_from_timestamps(&timestamps, 1_000_000, 0.3, 0.31).unwrap_err();
        assert!(err.to_string().contains("beyond the end"));
    }
}
