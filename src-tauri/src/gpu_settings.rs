use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuTelemetryConfig {
    pub namespace: String,
    pub service: String,
    pub port: String,
    pub cluster_label: Option<String>,
    pub cluster_value: Option<String>,
    pub single_cluster_acknowledged: bool,
}
fn dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
}
pub fn validate_config(c: &GpuTelemetryConfig) -> AppResult<()> {
    let valid_port = (c.port.bytes().all(|b| b.is_ascii_digit())
        && c.port.parse::<u16>().is_ok_and(|n| n > 0))
        || (dns_label(&c.port)
            && c.port.len() <= 15
            && c.port.bytes().any(|b| b.is_ascii_lowercase())
            && !c.port.contains("--"));
    if !dns_label(&c.namespace) || !dns_label(&c.service) || !valid_port {
        return Err(AppError::K8s(
            "Telemetry namespace, Service, and port must be valid Kubernetes names.".into(),
        ));
    }
    match (&c.cluster_label,&c.cluster_value) {
 (Some(label),Some(value)) if !label.is_empty() && label.len()<=128 && (label.as_bytes()[0].is_ascii_alphabetic() || label.starts_with('_')) && label.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_') && value.len()<=1024 => (),
 (None,None) if c.single_cluster_acknowledged => (),
 _ => return Err(AppError::K8s("Set an exact cluster label and value, or acknowledge a single-cluster telemetry source.".into()))
 }
    Ok(())
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    version: u8,
    configs: BTreeMap<String, GpuTelemetryConfig>,
}
pub struct GpuSettings {
    path: Option<PathBuf>,
    configs: Mutex<BTreeMap<String, GpuTelemetryConfig>>,
    failed: bool,
}
fn storage_error() -> AppError {
    AppError::Internal("GPU telemetry settings could not be loaded or saved. Repair the settings file and restart Lumen.".into())
}
impl GpuSettings {
    pub fn load(path: PathBuf) -> Self {
        let loaded = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Preferences>(&bytes)
                .ok()
                .filter(|p| {
                    p.version == 1
                        && p.configs.iter().all(|(context, c)| {
                            !context.trim().is_empty() && validate_config(c).is_ok()
                        })
                }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Preferences {
                version: 1,
                ..Default::default()
            }),
            Err(_) => None,
        };
        let failed = loaded.is_none();
        Self {
            path: Some(path),
            configs: Mutex::new(loaded.unwrap_or_default().configs),
            failed,
        }
    }
    pub fn unavailable() -> Self {
        Self {
            path: None,
            configs: Mutex::new(BTreeMap::new()),
            failed: true,
        }
    }
    pub fn get(&self, context: &str) -> Option<GpuTelemetryConfig> {
        self.configs.lock().ok()?.get(context).cloned()
    }
    pub fn ensure_available(&self) -> AppResult<()> {
        if self.failed {
            Err(storage_error())
        } else {
            Ok(())
        }
    }
    pub fn set(&self, context: &str, config: Option<GpuTelemetryConfig>) -> AppResult<()> {
        self.ensure_available()?;
        if context.trim().is_empty() {
            return Err(AppError::K8s(
                "An explicit cluster context is required.".into(),
            ));
        }
        if let Some(c) = &config {
            validate_config(c)?;
        }
        let mut current = self.configs.lock().map_err(|_| storage_error())?;
        let mut next = current.clone();
        if let Some(c) = config {
            next.insert(context.into(), c);
        } else {
            next.remove(context);
        }
        let path = self.path.as_ref().ok_or_else(storage_error)?;
        let parent = path.parent().ok_or_else(storage_error)?;
        std::fs::create_dir_all(parent).map_err(|_| storage_error())?;
        let temp = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = (|| -> std::io::Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(&serde_json::to_vec(&Preferences {
                version: 1,
                configs: next.clone(),
            })?)?;
            file.sync_all()?;
            std::fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(storage_error());
        }
        *current = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> GpuTelemetryConfig {
        GpuTelemetryConfig {
            namespace: "monitoring".into(),
            service: "prometheus".into(),
            port: "9090".into(),
            cluster_label: None,
            cluster_value: None,
            single_cluster_acknowledged: true,
        }
    }
    fn path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "lumen-gpu-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("settings.json")
    }
    #[test]
    fn gpu_settings_validation() {
        for invalid in ["prometheus/../../secrets", "", "Upper", "a.b", "-bad"] {
            let mut c = config();
            c.service = invalid.into();
            assert!(validate_config(&c).is_err());
        }
        for invalid in ["", "Upper", "-bad"] {
            let mut c = config();
            c.namespace = invalid.into();
            assert!(validate_config(&c).is_err());
        }
        for invalid in [
            "0",
            "65536",
            "+9090",
            "abc/def",
            "UPPER",
            "http--web",
            "abcdefghijklmnop",
        ] {
            let mut c = config();
            c.port = invalid.into();
            assert!(validate_config(&c).is_err());
        }
        for valid in ["1", "65535", "http-web", "123abc"] {
            let mut c = config();
            c.port = valid.into();
            assert!(validate_config(&c).is_ok());
        }
        let mut c = config();
        c.single_cluster_acknowledged = false;
        assert!(validate_config(&c).is_err());
        c.cluster_label = Some("cluster".into());
        c.cluster_value = Some("a\"} or vector(1)\n雪+&%".into());
        assert!(validate_config(&c).is_ok());
        c.cluster_label = Some("bad-name".into());
        assert!(validate_config(&c).is_err());
    }
    #[test]
    fn gpu_settings_context_isolation_and_persistence() {
        let p = path();
        let s = GpuSettings::load(p.clone());
        s.set("a", Some(config())).unwrap();
        assert_eq!(s.get("a"), Some(config()));
        assert!(s.get("b").is_none());
        assert_eq!(GpuSettings::load(p.clone()).get("a"), Some(config()));
        s.set("a", None).unwrap();
        assert!(GpuSettings::load(p.clone()).get("a").is_none());
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
    #[test]
    fn gpu_settings_failed_load_and_write_are_errors() {
        let p = path();
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "bad").unwrap();
        assert!(GpuSettings::load(p.clone())
            .set("a", Some(config()))
            .is_err());
        std::fs::remove_file(&p).unwrap();
        let s = GpuSettings::load(p.clone());
        std::fs::create_dir(&p).unwrap();
        assert!(s.set("a", Some(config())).is_err());
        assert!(s.get("a").is_none());
        assert!(GpuSettings::unavailable().set("a", Some(config())).is_err());
        std::fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }
}
