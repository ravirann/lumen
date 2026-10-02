//! Shared app state for Kubernetes clients, streams, port forwards, and pod attaches.

use crate::k8s::client::K8sState;
use crate::k8s::exec::AttachRegistry;
use crate::k8s::portforward::ForwardRegistry;
use std::sync::Arc;

pub struct AppState {
    pub gpu_settings: Arc<crate::gpu_settings::GpuSettings>,
    pub protection: Arc<crate::protection::ContextProtectionPolicy>,
    pub k8s: Arc<K8sState>,
    pub forwards: Arc<ForwardRegistry>,
    pub attachments: Arc<AttachRegistry>,
}

impl AppState {
    pub fn with_config_dir(path: std::path::PathBuf) -> Self {
        Self {
            gpu_settings: Arc::new(crate::gpu_settings::GpuSettings::load(
                path.join("gpu-telemetry.json"),
            )),
            protection: Arc::new(crate::protection::ContextProtectionPolicy::load(
                path.join("context-protection.json"),
            )),
            ..Self::new()
        }
    }

    pub fn new() -> Self {
        Self {
            gpu_settings: Arc::new(crate::gpu_settings::GpuSettings::unavailable()),
            protection: Arc::new(crate::protection::ContextProtectionPolicy::unavailable()),
            k8s: K8sState::new(),
            forwards: ForwardRegistry::new(),
            attachments: AttachRegistry::new(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
