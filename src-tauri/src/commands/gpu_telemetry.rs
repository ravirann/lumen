//! Explicit-context optional telemetry commands.
use crate::{
    error::{AppError, AppResult},
    gpu_settings::GpuTelemetryConfig,
    k8s::gpu_telemetry::{self, GpuTelemetryCapabilities},
    state::AppState,
};
fn context_required(context: &str) -> AppResult<()> {
    if context.trim().is_empty() {
        Err(AppError::K8s(
            "An explicit cluster context is required.".into(),
        ))
    } else {
        Ok(())
    }
}
#[tauri::command]
pub fn gpu_telemetry_config_get(
    context: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<Option<GpuTelemetryConfig>> {
    context_required(&context)?;
    state.gpu_settings.ensure_available()?;
    Ok(state.gpu_settings.get(&context))
}
#[tauri::command]
pub fn gpu_telemetry_config_set(
    context: String,
    config: Option<GpuTelemetryConfig>,
    state: tauri::State<'_, AppState>,
) -> AppResult<()> {
    context_required(&context)?;
    state.gpu_settings.set(&context, config)
}
#[tauri::command]
pub async fn gpu_telemetry_capabilities(
    context: String,
    state: tauri::State<'_, AppState>,
) -> AppResult<GpuTelemetryCapabilities> {
    context_required(&context)?;
    state.gpu_settings.ensure_available()?;
    let config = state.gpu_settings.get(&context).ok_or_else(|| {
        AppError::K8s("Configure a telemetry Service for this context first.".into())
    })?;
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state.k8s.client_for(&context),
    )
    .await
    .map_err(|_| AppError::K8s("Connecting to the selected context timed out.".into()))?
    .map_err(|_| AppError::K8s("Connecting to the selected context failed.".into()))?;
    gpu_telemetry::capabilities(&client, &config).await
}
#[tauri::command]
pub async fn gpu_telemetry_history(
    context: String,
    namespace: Option<String>,
    window_seconds: u64,
    end_seconds: f64,
    state: tauri::State<'_, AppState>,
) -> AppResult<gpu_telemetry::GpuHistory> {
    context_required(&context)?;
    state.gpu_settings.ensure_available()?;
    let config = state.gpu_settings.get(&context).ok_or_else(|| {
        AppError::K8s("Configure a telemetry Service for this context first.".into())
    })?;
    let client = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state.k8s.client_for(&context),
    )
    .await
    .map_err(|_| AppError::K8s("Connecting to the selected context timed out.".into()))?
    .map_err(|_| AppError::K8s("Connecting to the selected context failed.".into()))?;
    gpu_telemetry::history(
        &client,
        &config,
        namespace.as_deref(),
        window_seconds,
        end_seconds,
    )
    .await
}
