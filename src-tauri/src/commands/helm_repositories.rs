//! Repository configuration belongs to the local Helm installation, not a cluster.
use crate::{error::AppResult, k8s::helm_cli};

#[tauri::command]
pub async fn helm_repository_list() -> AppResult<Vec<helm_cli::HelmRepository>> {
    helm_cli::list_repositories().await
}

#[tauri::command]
pub async fn helm_repository_add(name: String, url: String) -> AppResult<()> {
    helm_cli::add_repository(&name, &url).await
}

#[tauri::command]
pub async fn helm_repository_remove(name: String) -> AppResult<()> {
    helm_cli::remove_repository(&name).await
}

#[tauri::command]
pub async fn helm_repository_update() -> AppResult<()> {
    helm_cli::update_repositories().await
}
