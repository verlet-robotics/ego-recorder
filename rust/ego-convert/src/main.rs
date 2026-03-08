mod commands;
mod progress;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ego-convert", about = "Convert .egorec v2 recordings to ML dataset formats")]
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
    }

    Ok(())
}
