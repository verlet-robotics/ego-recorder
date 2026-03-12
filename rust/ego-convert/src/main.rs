mod commands;
mod progress;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ego-convert", about = "Convert .egorec v2 recordings to other formats")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Export to RLDS TFRecord format
    Rlds {
        /// .egorec v2 file paths
        #[arg(required = true)]
        files: Vec<String>,
        /// Output directory (default: same dir as first input file, with _rlds suffix)
        #[arg(short, long)]
        output: Option<String>,
        /// Dataset name (default: session name from first file)
        #[arg(long)]
        name: Option<String>,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
        /// Dataset name (from dataset.json manifest)
        #[arg(long)]
        dataset_name: Option<String>,
        /// Dataset description (from dataset.json manifest)
        #[arg(long)]
        dataset_description: Option<String>,
        /// Comma-separated dataset tags (from dataset.json manifest)
        #[arg(long)]
        dataset_tags: Option<String>,
    },
    /// Export to LeRobot v3 dataset format
    Lerobot {
        /// .egorec v2 file paths
        #[arg(required = true)]
        files: Vec<String>,
        /// Output directory (default: same dir as first input file, with _lerobot suffix)
        #[arg(short, long)]
        output: Option<String>,
        /// Dataset name / repo_id (default: session name from file)
        #[arg(long)]
        name: Option<String>,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
        /// Create separate dataset per recording (default: merge into one)
        #[arg(long)]
        separate: bool,
        /// Dataset name (from dataset.json manifest)
        #[arg(long)]
        dataset_name: Option<String>,
        /// Dataset description (from dataset.json manifest)
        #[arg(long)]
        dataset_description: Option<String>,
        /// Comma-separated dataset tags (from dataset.json manifest)
        #[arg(long)]
        dataset_tags: Option<String>,
    },
    /// Extract browser-playable MP4 from .egorec files (full decode + re-encode)
    Mp4 {
        /// .egorec v2 file paths
        #[arg(required = true)]
        files: Vec<String>,
        /// Output directory (default: same dir as input file)
        #[arg(short, long)]
        output: Option<String>,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Fast proxy MP4 via H.264 remux (no decode/encode, requires ffmpeg in PATH)
    Proxy {
        /// .egorec v2 file paths
        #[arg(required = true)]
        files: Vec<String>,
        /// Output directory (default: same dir as input file)
        #[arg(short, long)]
        output: Option<String>,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Import MP4 video + depth PNGs into .egorec format
    Import {
        /// Path to RGB video (MP4)
        #[arg(long)]
        video: String,
        /// Directory containing depth PNGs (1-indexed: 1.png, 2.png, ...)
        #[arg(long)]
        depth_dir: String,
        /// Output .egorec file path
        #[arg(short, long)]
        output: String,
        /// Output width (default: 1280)
        #[arg(long, default_value = "1280")]
        width: u32,
        /// Output height (default: 720)
        #[arg(long, default_value = "720")]
        height: u32,
        /// Target FPS (default: 30)
        #[arg(long, default_value = "30")]
        fps: u32,
        /// Session name for the recording
        #[arg(long)]
        session_name: Option<String>,
        /// Suppress progress output
        #[arg(short, long)]
        quiet: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Rlds {
            files,
            output,
            name,
            quiet,
            dataset_name,
            dataset_description,
            dataset_tags,
        } => {
            commands::rlds::run(
                &files,
                output.as_deref(),
                name.as_deref(),
                quiet,
                dataset_name.as_deref(),
                dataset_description.as_deref(),
                dataset_tags.as_deref(),
            )?;
        }
        Commands::Lerobot {
            files,
            output,
            name,
            quiet,
            separate,
            dataset_name,
            dataset_description,
            dataset_tags,
        } => {
            commands::lerobot::run(
                &files,
                output.as_deref(),
                name.as_deref(),
                quiet,
                separate,
                dataset_name.as_deref(),
                dataset_description.as_deref(),
                dataset_tags.as_deref(),
            )?;
        }
        Commands::Mp4 {
            files,
            output,
            quiet,
        } => {
            commands::mp4::run(&files, output.as_deref(), quiet)?;
        }
        Commands::Proxy {
            files,
            output,
            quiet,
        } => {
            commands::proxy::run(&files, output.as_deref(), quiet)?;
        }
        Commands::Import {
            video,
            depth_dir,
            output,
            width,
            height,
            fps,
            session_name,
            quiet,
        } => {
            commands::import::run(
                &video,
                &depth_dir,
                &output,
                width,
                height,
                fps,
                session_name.as_deref(),
                quiet,
            )?;
        }
    }

    Ok(())
}
