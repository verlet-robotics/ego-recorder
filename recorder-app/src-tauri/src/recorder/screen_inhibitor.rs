/// Inhibit the desktop screensaver / display blanking via the
/// org.freedesktop.ScreenSaver D-Bus interface (session bus).
///
/// Returns a cookie that must be passed to `uninhibit_screen()` to release.

/// Acquire a screensaver inhibitor on the session bus.
pub async fn inhibit_screen() -> Result<u32, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| format!("Session D-Bus connection failed: {}", e))?;

    let reply = connection
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "Inhibit",
            &("ego-recorder-app", "Keep display on while app is running"),
        )
        .await
        .map_err(|e| format!("ScreenSaver Inhibit call failed: {}", e))?;

    let cookie: u32 = reply
        .body()
        .deserialize()
        .map_err(|e| format!("Failed to get inhibit cookie: {}", e))?;

    log::info!("Acquired screensaver inhibitor (cookie={})", cookie);
    Ok(cookie)
}

/// Release the screensaver inhibitor.
pub async fn uninhibit_screen(cookie: u32) {
    let Ok(connection) = zbus::Connection::session().await else {
        log::warn!("Failed to connect to session D-Bus to release screen inhibitor");
        return;
    };

    match connection
        .call_method(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            Some("org.freedesktop.ScreenSaver"),
            "UnInhibit",
            &(cookie,),
        )
        .await
    {
        Ok(_) => log::info!("Released screensaver inhibitor (cookie={})", cookie),
        Err(e) => log::warn!("Failed to release screen inhibitor: {}", e),
    }
}
