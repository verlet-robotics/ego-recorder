/// Progress bar helpers using indicatif, matching Python tqdm output style.
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Instant;

pub struct ExportProgress {
    bar: ProgressBar,
    bytes_processed: u64,
    start: Instant,
}

impl ExportProgress {
    pub fn new(total_frames: u64, filename: &str) -> Self {
        let bar = ProgressBar::new(total_frames);
        bar.set_style(
            ProgressStyle::with_template(
                "{prefix} [{bar:40.cyan/blue}] {pos}/{len} frames ({msg})",
            )
            .unwrap()
            .progress_chars("=> "),
        );
        bar.set_prefix(filename.to_string());

        Self {
            bar,
            bytes_processed: 0,
            start: Instant::now(),
        }
    }

    pub fn update(&mut self, frame_bytes: u64) {
        self.bytes_processed += frame_bytes;
        self.bar.inc(1);

        let elapsed = self.start.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            let mb_per_s = (self.bytes_processed as f64 / 1e6) / elapsed;
            self.bar.set_message(format!("{:.1} MB/s", mb_per_s));
        }
    }

    pub fn finish(&self) {
        self.bar.finish();
    }
}
