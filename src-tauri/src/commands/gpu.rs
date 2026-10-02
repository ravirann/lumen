//! Explicit-context read-only GPU scheduling IPC.
use crate::{
    error::{AppError, AppResult},
    k8s::{devices, gpu_inventory, gpu_scheduling, gpu_types::SchedulingSnapshot},
    state::AppState,
};
use std::time::Duration;
pub fn valid_target(namespace: &str, pod: &str, uid: &str) -> bool {
    !namespace.is_empty()
        && devices::valid_namespace(namespace)
        && !pod.is_empty()
        && pod.len() <= 253
        && pod
            .split('.')
            .all(|part| !part.is_empty() && devices::valid_namespace(part))
        && !uid.is_empty()
        && uid.len() <= 128
        && uid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
#[tauri::command]
pub async fn gpu_scheduling_snapshot(
    context: String,
    namespace: String,
    pod: String,
    expected_uid: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<SchedulingSnapshot> {
    if context.trim().is_empty() {
        return Err(AppError::K8s(
            "An explicit cluster context is required.".into(),
        ));
    }
    if !valid_target(&namespace, &pod, &expected_uid) {
        return Err(AppError::K8s(
            "A valid namespace, pod name, and expected pod UID are required.".into(),
        ));
    }
    let client = tokio::time::timeout(Duration::from_secs(10), state.k8s.client_for(&context))
        .await
        .map_err(|_| AppError::K8s("Connecting to the selected context timed out.".into()))?
        .map_err(|_| AppError::K8s("Could not connect to the selected context.".into()))?;
    gpu_scheduling::snapshot(&client, &namespace, &pod, &expected_uid).await
}
/// The UI must pin both the context and a concrete namespace for workload totals.
#[tauri::command]
pub async fn gpu_inventory_snapshot(
    context: String,
    namespace: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<gpu_inventory::GpuInventorySnapshot> {
    if context.trim().is_empty() {
        return Err(AppError::K8s(
            "An explicit cluster context is required.".into(),
        ));
    }
    if namespace.is_empty() || !devices::valid_namespace(&namespace) {
        return Err(AppError::K8s(
            "A valid explicit namespace is required.".into(),
        ));
    }
    let client = tokio::time::timeout(Duration::from_secs(10), state.k8s.client_for(&context))
        .await
        .map_err(|_| AppError::K8s("Connecting to the selected context timed out.".into()))?
        .map_err(|_| AppError::K8s("Could not connect to the selected context.".into()))?;
    Ok(gpu_inventory::snapshot(&client, &namespace).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_name_segments_and_invalid_uid_paths() {
        assert!(valid_target("ns", "train.worker", "abc-123"));
        for pod in ["a..b", ".a", "a.", "a/b", ""] {
            assert!(!valid_target("ns", pod, "u"));
        }
        assert!(!valid_target("", "p", "u"));
        assert!(!valid_target("ns", "p", "a/b"));
    }
}
