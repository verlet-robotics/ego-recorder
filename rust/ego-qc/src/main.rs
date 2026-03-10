mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ego-qc", about = "Quality control tools for .egorec v2 recordings")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Validate structural integrity of .egorec files
    Validate {
        /// .egorec file or directory paths
        #[arg(required = true)]
        paths: Vec<String>,
        /// Suppress OK output
        #[arg(short, long)]
        quiet: bool,
    },
    /// Analyze episodes for activity (report only, never destructive)
    Analyze {
        /// .egorec file or directory paths
        #[arg(required = true)]
        paths: Vec<String>,
        /// Verbose output with reasons
        #[arg(short, long)]
        verbose: bool,
        /// Write JSON report to file
        #[arg(long)]
        report: Option<String>,
        /// Activity threshold k (median + k*MAD)
        #[arg(long)]
        activity_k: Option<f32>,
        /// Station baseline profile (from calibrate --save-profile)
        #[arg(long)]
        profile: Option<String>,
    },
    /// Dump per-episode features for threshold tuning
    Calibrate {
        /// .egorec file or directory paths
        #[arg(required = true)]
        paths: Vec<String>,
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
        /// Output format: csv or json
        #[arg(long, default_value = "csv")]
        format: String,
        /// Save aggregated station profile to JSON file
        #[arg(long)]
        save_profile: Option<String>,
    },
    /// Move low-activity episodes to .pruned/ (dry-run by default)
    Prune {
        /// .egorec file or directory paths
        #[arg(required = true)]
        paths: Vec<String>,
        /// Actually move files (default: dry run)
        #[arg(long)]
        apply: bool,
        /// Activity score threshold for PRUNE_SUGGESTED (default: prune all suggested)
        #[arg(long)]
        threshold: Option<f32>,
        /// Station baseline profile (from calibrate --save-profile)
        #[arg(long)]
        profile: Option<String>,
    },
    /// Extract active segments from recordings
    Splice {
        /// .egorec file or directory paths
        #[arg(required = true)]
        paths: Vec<String>,
        /// Minimum idle gap in seconds before splitting (default: 10)
        #[arg(long)]
        min_gap: Option<f64>,
        /// Minimum segment duration in seconds (default: 2)
        #[arg(long)]
        min_duration: Option<f64>,
        /// Move originals to .pruned/ after splicing
        #[arg(long)]
        replace_original: bool,
        /// Station baseline profile (from calibrate --save-profile)
        #[arg(long)]
        profile: Option<String>,
    },
    /// Extract MP4 video (RGB + depth visualization) from .egorec files
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
    /// Restore a pruned/spliced original from .pruned/
    Restore {
        /// Dataset directory
        #[arg(required = true)]
        dataset: String,
        /// Filename to restore
        #[arg(required = true)]
        filename: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Validate { paths, quiet } => {
            commands::validate::run(&paths, quiet)?;
        }
        Commands::Analyze {
            paths,
            verbose,
            report,
            activity_k,
            profile,
        } => {
            commands::analyze::run(&paths, verbose, report.as_deref(), activity_k, profile.as_deref())?;
        }
        Commands::Calibrate {
            paths,
            output,
            format,
            save_profile,
        } => {
            commands::calibrate::run(&paths, output.as_deref(), &format, save_profile.as_deref())?;
        }
        Commands::Prune {
            paths,
            apply,
            threshold,
            profile,
        } => {
            commands::prune::run(&paths, apply, threshold, profile.as_deref())?;
        }
        Commands::Splice {
            paths,
            min_gap,
            min_duration,
            replace_original,
            profile,
        } => {
            let min_gap_frames = min_gap.map(|s| (s * 30.0) as u64);
            let min_dur_frames = min_duration.map(|s| (s * 30.0) as u64);
            commands::splice::run(&paths, min_gap_frames, min_dur_frames, replace_original, profile.as_deref())?;
        }
        Commands::Mp4 {
            files,
            output,
            quiet,
        } => {
            commands::mp4::run(&files, output.as_deref(), quiet)?;
        }
        Commands::Restore { dataset, filename } => {
            commands::restore::run(&dataset, &filename)?;
        }
    }

    Ok(())
}
