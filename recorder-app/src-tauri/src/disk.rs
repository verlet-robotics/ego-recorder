use nix::sys::statvfs::statvfs;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f64,
    pub free_mb: u64,
}

pub fn get_disk_info(path: &str) -> Result<DiskInfo, String> {
    let stat = statvfs(path).map_err(|e| format!("statvfs failed: {}", e))?;
    let total = stat.blocks() * stat.fragment_size() as u64;
    let free = stat.blocks_available() * stat.fragment_size() as u64;
    let used = total.saturating_sub(free);
    let usage = if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    Ok(DiskInfo {
        total_bytes: total,
        free_bytes: free,
        used_bytes: used,
        usage_percent: usage,
        free_mb: free / (1024 * 1024),
    })
}
