use std::os::fd::AsRawFd;

/// Acquire a D-Bus inhibitor lock that blocks lid-close, sleep, and idle.
/// Returns the file descriptor on success.
pub async fn acquire_inhibitor() -> Result<i32, String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;

    let reply = connection
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &(
                "handle-lid-switch:sleep:idle",
                "ego-recorder-app",
                "Recording in progress",
                "block",
            ),
        )
        .await
        .map_err(|e| format!("Inhibit call failed: {}", e))?;

    let fd: zbus::zvariant::OwnedFd = reply
        .body()
        .deserialize()
        .map_err(|e| format!("Failed to get fd: {}", e))?;

    // Duplicate the fd so we own it after OwnedFd drops
    let raw = fd.as_raw_fd();
    let duped = nix::unistd::dup(raw).map_err(|e| format!("dup failed: {}", e))?;

    log::info!("Acquired D-Bus inhibitor lock (fd={})", duped);
    Ok(duped)
}

/// Release the inhibitor lock by closing the file descriptor.
pub fn release_inhibitor(fd: i32) {
    if fd >= 0 {
        let _ = nix::unistd::close(fd);
        log::info!("Released D-Bus inhibitor lock (fd={})", fd);
    }
}
