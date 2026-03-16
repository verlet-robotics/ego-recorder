#[tauri::command]
pub async fn open_directory() -> Result<Option<String>, String> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Select Directory")
        .pick_folder()
        .await;
    Ok(handle.map(|h| h.path().to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn select_file(title: String) -> Result<Option<String>, String> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title(&title)
        .pick_file()
        .await;
    Ok(handle.map(|h| h.path().to_string_lossy().to_string()))
}
