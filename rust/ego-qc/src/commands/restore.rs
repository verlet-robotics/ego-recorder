use anyhow::Result;
use std::fs;
use std::path::Path;

use super::prune::{load_manifest, save_manifest, write_audit_entry, AuditEntry, EpisodeEntry};

pub fn run(dataset_path: &str, filename: &str) -> Result<()> {
    let dir = Path::new(dataset_path);
    let pruned_dir = dir.join(".pruned");
    let source = pruned_dir.join(filename);

    if !source.exists() {
        anyhow::bail!("{} not found in .pruned/", filename);
    }

    let dest = dir.join(filename);
    if dest.exists() {
        anyhow::bail!("{} already exists in dataset directory", filename);
    }

    // Move file back
    fs::rename(&source, &dest)?;
    println!("RESTORED  .pruned/{} -> {}", filename, filename);

    // Write audit entry
    write_audit_entry(
        &pruned_dir,
        &AuditEntry::Restore {
            timestamp: chrono::Utc::now().to_rfc3339(),
            source: format!(".pruned/{}", filename),
            destination: filename.to_string(),
        },
    )?;

    // Update manifest — re-add the episode
    if let Ok(mut manifest) = load_manifest(dir) {
        // Check if already in manifest
        if !manifest.episodes.iter().any(|e| e.filename == filename) {
            // Read metadata from the file
            let config = egorec::ScanConfig::default();
            let (duration_s, frames, session_name) =
                match egorec::EgorecScanner::scan(&dest, &config) {
                    Ok(summary) => {
                        let mut reader_file =
                            std::io::BufReader::new(fs::File::open(&dest)?);
                        let header = egorec::FileHeader::read_from(&mut reader_file)?;
                        (
                            summary.duration_us as f64 / 1e6,
                            summary.total_frames,
                            header.session_name_str().to_string(),
                        )
                    }
                    Err(_) => (0.0, 0, String::new()),
                };

            manifest.episodes.push(EpisodeEntry {
                filename: filename.to_string(),
                session_name,
                recorded_at: String::new(),
                duration_s,
                frames,
            });
            save_manifest(dir, &manifest)?;
            println!("Manifest updated (episode restored)");
        }
    }

    Ok(())
}
