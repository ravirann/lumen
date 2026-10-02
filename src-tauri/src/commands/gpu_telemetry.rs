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
    capability_deadline(gpu_telemetry::REQUEST_TIMEOUT, async {
        let client = state
            .k8s
            .client_for(&context)
            .await
            .map_err(|_| AppError::K8s("Connecting to the selected context failed.".into()))?;
        gpu_telemetry::capabilities(&client, &config).await
    })
    .await
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
    history_deadline(gpu_telemetry::REQUEST_TIMEOUT, async {
        let client = state
            .k8s
            .client_for(&context)
            .await
            .map_err(|_| AppError::K8s("Connecting to the selected context failed.".into()))?;
        gpu_telemetry::history(
            &client,
            &config,
            namespace.as_deref(),
            window_seconds,
            end_seconds,
        )
        .await
    })
    .await
}
async fn history_deadline<T>(
    duration: std::time::Duration,
    operation: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    tokio::time::timeout(duration, operation)
        .await
        .map_err(|_| {
            AppError::K8s(
                "Telemetry history request timed out after 15 seconds, including connection setup."
                    .into(),
            )
        })?
}
async fn capability_deadline<T>(
    duration: std::time::Duration,
    operation: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    tokio::time::timeout(duration, operation).await.map_err(|_| AppError::K8s(
        "Telemetry capability request timed out after 15 seconds, including connection setup and access review.".into()
    ))?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn gpu_capability_deadline_spans_client_review_and_query() {
        let result = capability_deadline(std::time::Duration::from_millis(20), async {
            // Each phase meets its own budget; the composed operation does not.
            for _ in 0..3 {
                tokio::time::sleep(std::time::Duration::from_millis(9)).await;
            }
            Ok(())
        })
        .await;
        assert!(result.unwrap_err().to_string().contains("15 seconds"));
    }
    #[tokio::test]
    async fn gpu_history_deadline_spans_setup_and_query() {
        let result = history_deadline(std::time::Duration::from_millis(20), async {
            tokio::time::sleep(std::time::Duration::from_millis(12)).await;
            tokio::time::sleep(std::time::Duration::from_millis(12)).await;
            Ok(())
        })
        .await;
        assert!(result.unwrap_err().to_string().contains("15 seconds"));
    }
}
