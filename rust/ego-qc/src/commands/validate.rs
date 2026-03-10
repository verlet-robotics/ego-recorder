use anyhow::Result;
use egorec::EgorecScanner;
use std::path::Path;

pub fn run(paths: &[String], quiet: bool) -> Result<()> {
    let files = collect_egorec_files(paths)?;
    if files.is_empty() {
        anyhow::bail!("no .egorec files found");
    }

    let mut all_valid = true;

    for file in &files {
        let path = Path::new(file);
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        match EgorecScanner::validate(path) {
            Ok(result) => {
                if result.valid {
                    if !quiet {
                        println!(
                            "  OK  {}  ({} frames, {:.1}s, {} index entries)",
                            name,
                            result.total_frames,
                            result.duration_us as f64 / 1e6,
                            result.index_entries
                        );
                    }
                } else {
                    all_valid = false;
                    println!("FAIL  {}", name);
                    for err in &result.errors {
                        println!("        error: {}", err);
                    }
                }
                for warn in &result.warnings {
                    if !quiet {
                        println!("        warn: {}", warn);
                    }
                }
            }
            Err(e) => {
                all_valid = false;
                println!("FAIL  {}  ({})", name, e);
            }
        }
    }

    let total = files.len();
    let passed = if all_valid {
        total
    } else {
        files
            .iter()
            .filter(|f| {
                EgorecScanner::validate(Path::new(f))
                    .map(|r| r.valid)
                    .unwrap_or(false)
            })
            .count()
    };

    println!("\n{}/{} files passed validation", passed, total);

    if !all_valid {
        anyhow::bail!("validation failed");
    }
    Ok(())
}

pub fn collect_egorec_files(paths: &[String]) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            // Look for .egorec files in directory
            let mut entries: Vec<_> = std::fs::read_dir(path)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "egorec")
                        .unwrap_or(false)
                })
                .map(|e| e.path().to_string_lossy().to_string())
                .collect();
            entries.sort();
            files.extend(entries);
        } else if path.extension().map(|e| e == "egorec").unwrap_or(false) {
            files.push(p.clone());
        }
    }
    Ok(files)
}
