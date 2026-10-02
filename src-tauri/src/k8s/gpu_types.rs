//! Purpose-specific GPU evidence contracts. No arbitrary workload configuration.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Available,
    Unsupported,
    Forbidden,
    Error,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source<T> {
    pub state: SourceState,
    pub complete: bool,
    pub captured_at: String,
    pub items: Vec<T>,
    pub message: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub api_version: String,
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Explanation {
    pub stage: String,
    pub confidence: String,
    pub message: String,
    pub sources: Vec<ResourceRef>,
    pub captured_at: String,
    pub transition_time: Option<String>,
    pub observed_generation: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulingSnapshot {
    pub pod: ResourceRef,
    pub sources: HashMap<String, Source<Value>>,
    pub explanations: Vec<Explanation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuPod {
    pub resource: ResourceRef,
    pub phase: String,
    pub node_name: Option<String>,
    pub requests: HashMap<String, Option<i64>>,
    pub owners: Vec<ResourceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuNode {
    pub resource: ResourceRef,
    pub allocatable: HashMap<String, Option<i64>>,
    pub gpu_labels: HashMap<String, String>,
}
pub fn resource_ref(v: &Value) -> ResourceRef {
    let s = |p| {
        v.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    ResourceRef {
        api_version: s("/apiVersion"),
        kind: s("/kind"),
        namespace: v
            .pointer("/metadata/namespace")
            .and_then(Value::as_str)
            .map(str::to_owned),
        name: s("/metadata/name"),
        uid: s("/metadata/uid"),
    }
}
