use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct ContextInfo {
    pub name: String,
    pub cluster: String,
    pub user: String,
    pub namespace: Option<String>,
    pub is_current: bool,
    pub is_prod: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkloadKind {
    Node,
    Namespace,
    Event,
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
    ReplicationController,
    CronJob,
    Job,
    Pod,
    Service,
    Ingress,
    Endpoint,
    EndpointSlice,
    ConfigMap,
    Secret,
    ServiceAccount,
    Role,
    RoleBinding,
    ClusterRole,
    ClusterRoleBinding,
    NetworkPolicy,
    #[serde(rename = "persistentvolumeclaim")]
    PersistentVolumeClaim,
    // Cluster-scoped — list_workloads ignores the namespace arg for these
    // and uses Api::all instead.
    #[serde(rename = "persistentvolume")]
    PersistentVolume,
    #[serde(rename = "storageclass")]
    StorageClass,
    #[serde(rename = "csidriver")]
    CsiDriver,
    #[serde(rename = "csinode")]
    CsiNode,
    #[serde(rename = "volumeattributesclass")]
    VolumeAttributesClass,
    #[serde(rename = "podtemplate")]
    PodTemplate,
    #[serde(rename = "ingressclass")]
    IngressClass,
    // Namespaced.
    #[serde(rename = "resourcequota")]
    ResourceQuota,
    #[serde(rename = "horizontalpodautoscaler")]
    HorizontalPodAutoscaler,
    #[serde(rename = "verticalpodautoscaler")]
    VerticalPodAutoscaler,
    // PR D+1 long-tail kinds.
    #[serde(rename = "limitrange")]
    LimitRange,
    #[serde(rename = "poddisruptionbudget")]
    PodDisruptionBudget,
    #[serde(rename = "priorityclass")]
    PriorityClass,
    #[serde(rename = "runtimeclass")]
    RuntimeClass,
    Lease,
    #[serde(rename = "controllerrevision")]
    ControllerRevision,
    #[serde(rename = "mutatingwebhookconfiguration")]
    MutatingWebhookConfiguration,
    #[serde(rename = "validatingwebhookconfiguration")]
    ValidatingWebhookConfiguration,
    #[serde(rename = "gatewayclass")]
    GatewayClass,
    Gateway,
    #[serde(rename = "httproute")]
    HttpRoute,
    #[serde(rename = "grpcroute")]
    GrpcRoute,
    #[serde(rename = "jobset")]
    JobSet,
    #[serde(rename = "apiservice")]
    ApiService,
    #[serde(rename = "certificatesigningrequest")]
    CertificateSigningRequest,
    #[serde(rename = "customresourcedefinition")]
    CustomResourceDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    Healthy,
    Degraded,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkloadSummary {
    pub kind: WorkloadKind,
    pub name: String,
    pub namespace: String,
    pub ready: String,
    pub age_seconds: i64,
    pub health: Health,
    pub labels: std::collections::BTreeMap<String, String>,
    // ─── Pod-only enrichment (None for non-pod kinds). ───
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_ready_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controlled_by: Option<OwnerRefLite>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qos_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_milli: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_phase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerRefLite {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceDetail {
    pub summary: WorkloadSummary,
    pub yaml: String,
    pub owner_refs: Vec<OwnerRefLite>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RbacRuleDetail {
    pub api_groups: Vec<String>,
    pub resources: Vec<String>,
    pub resource_names: Vec<String>,
    pub non_resource_urls: Vec<String>,
    pub verbs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RbacSubjectDetail {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RbacDetail {
    pub role_ref: Option<String>,
    pub subjects: Vec<RbacSubjectDetail>,
    pub rules: Vec<RbacRuleDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageDetail {
    pub phase: Option<String>,
    pub capacity: Option<String>,
    pub access_modes: Vec<String>,
    pub storage_class: Option<String>,
    pub volume_name: Option<String>,
    pub reclaim_policy: Option<String>,
    pub binding_mode: Option<String>,
    pub provisioner: Option<String>,
    pub allow_expansion: Option<bool>,
    pub claim_ref: Option<String>,
    pub parameters: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceInsightRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceInsightSection {
    pub title: String,
    pub rows: Vec<ResourceInsightRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceInsights {
    pub sections: Vec<ResourceInsightSection>,
}

// ─── Fleet ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct NodeSummary {
    pub name: String,
    pub roles: Vec<String>,
    pub version: String,
    pub ready: bool,
    pub os_image: String,
    pub arch: String,
    pub cpu_capacity_milli: i64,
    pub mem_capacity_bytes: i64,
    pub pods_capacity: i64,
    pub cpu_allocatable_milli: i64,
    pub mem_allocatable_bytes: i64,
    pub taints: Vec<String>,
    pub age_seconds: i64,
    pub cpu_usage_milli: Option<i64>,
    pub mem_usage_bytes: Option<i64>,
    /// Mirrors `spec.unschedulable`. `true` means the node is cordoned —
    /// kube-scheduler will not place new pods on it. Existing pods stay put
    /// until they are explicitly drained or rescheduled.
    pub unschedulable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetHealth {
    pub pods_total: i32,
    pub pods_ready: i32,
    pub pods_pending: i32,
    pub pods_failed: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct FleetCard {
    pub context: ContextInfo,
    pub reachable: bool,
    /// Connection failure when unreachable; incomplete inventory warning otherwise.
    pub error: Option<String>,
    pub server_version: Option<String>,
    pub node_count: i32,
    pub node_ready: i32,
    pub namespace_count: i32,
    pub workload_count: i32,
    pub health: FleetHealth,
    /// 0–100 aggregate CPU usage across nodes (from metrics-server if
    /// available, else from requests).
    pub cpu_percent: Option<f32>,
    /// 0–100 aggregate memory usage across nodes.
    pub mem_percent: Option<f32>,
    pub fetched_at_ms: i64,
    /// Best-effort cluster distribution (eks / gke / aks / openshift / k3s /
    /// minikube / kind / generic). Inferred from node labels and well-known
    /// system namespaces; `None` when nothing matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution: Option<String>,
}

// ─── CloudMap ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Namespace,
    Deployment,
    StatefulSet,
    DaemonSet,
    CronJob,
    Job,
    Pod,
    Service,
    Ingress,
    ConfigMap,
    Secret,
    PersistentVolumeClaim,
    Node,
    Hpa,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapNode {
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    pub namespace: Option<String>,
    pub health: Health,
    /// Relative "heat" 0–100 — for heatmap visualization. CPU or replica
    /// pressure, depending on kind.
    pub heat: f32,
    pub ready: Option<String>,
    pub labels: std::collections::BTreeMap<String, String>,
    pub replicas: Option<i32>,
    pub extra: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A service selects a pod via labels.
    Selects,
    /// An ingress routes to a service.
    Routes,
    /// A pod is owned by a replicaset/deployment/statefulset/daemonset/job.
    OwnedBy,
    /// A pod mounts a configmap/secret/pvc.
    Mounts,
    /// A hpa targets a workload.
    Scales,
    /// A pod is scheduled on a node.
    ScheduledOn,
}

#[derive(Debug, Clone, Serialize)]
pub struct MapEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudMap {
    pub context: String,
    pub nodes: Vec<MapNode>,
    pub edges: Vec<MapEdge>,
    pub fetched_at_ms: i64,
}

// ─── Network Debugger ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDebugSnapshot {
    pub loaded_namespaces: Vec<String>,
    pub unavailable: BTreeMap<String, String>,
    pub gateway_resources: Vec<serde_json::Value>,
    pub namespaces: Vec<NetworkNamespace>,
    pub pods: Vec<NetworkPod>,
    pub services: Vec<NetworkService>,
    pub endpoints: Vec<NetworkEndpoints>,
    pub endpoint_slices: Vec<NetworkEndpointSlice>,
    pub ingresses: Vec<NetworkIngress>,
    pub network_policies: Vec<NetworkPolicyResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkNamespace {
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPod {
    pub name: String,
    pub namespace: String,
    pub labels: BTreeMap<String, String>,
    pub ready: bool,
    pub phase: Option<String>,
    pub pod_ip: Option<String>,
    pub ports: Vec<NetworkPodPort>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPodPort {
    pub name: Option<String>,
    pub container_port: i32,
    pub protocol: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkService {
    pub name: String,
    pub namespace: String,
    pub r#type: Option<String>,
    pub selector: BTreeMap<String, String>,
    pub ports: Vec<NetworkServicePort>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkServicePort {
    pub name: Option<String>,
    pub protocol: Option<String>,
    pub port: i32,
    pub target_port: Option<IntOrStringValue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum IntOrStringValue {
    Int(i32),
    String(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkEndpoints {
    pub name: String,
    pub namespace: String,
    pub addresses: Vec<NetworkEndpointAddress>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEndpointSlice {
    pub name: String,
    pub namespace: String,
    pub service_name: String,
    pub ports: Vec<NetworkEndpointPort>,
    pub endpoints: Vec<NetworkEndpointAddress>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkEndpointPort {
    pub name: Option<String>,
    pub protocol: Option<String>,
    pub port: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkEndpointAddress {
    pub addresses: Vec<String>,
    pub ready: bool,
    pub target_ref: Option<NetworkEndpointRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkEndpointRef {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkIngress {
    pub name: String,
    pub namespace: String,
    pub class_name: Option<String>,
    pub rules: Vec<NetworkIngressRule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkIngressRule {
    pub host: Option<String>,
    pub paths: Vec<NetworkIngressBackend>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkIngressBackend {
    pub path: String,
    pub path_type: Option<String>,
    pub service_name: String,
    pub service_port: Option<IntOrStringValue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyResource {
    pub unsupported: bool,
    pub egress: Option<Vec<NetworkPolicyEgressRule>>,
    pub name: String,
    pub namespace: String,
    pub pod_selector: BTreeMap<String, String>,
    pub policy_types: Vec<String>,
    pub ingress: Vec<NetworkPolicyIngressRule>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkPolicyEgressRule {
    pub to: Vec<NetworkPolicyPeer>,
    pub ports: Vec<NetworkPolicyPort>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkPolicyIngressRule {
    pub from: Vec<NetworkPolicyPeer>,
    pub ports: Vec<NetworkPolicyPort>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyPeer {
    pub unsupported: bool,
    pub pod_selector: Option<BTreeMap<String, String>>,
    pub namespace_selector: Option<BTreeMap<String, String>>,
    pub ip_block: Option<NetworkPolicyIpBlock>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkPolicyIpBlock {
    pub cidr: String,
    pub except: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkPolicyPort {
    pub end_port: Option<i32>,
    pub protocol: Option<String>,
    pub port: Option<IntOrStringValue>,
}

// ─── Security / DevSec ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub title: String,
    pub severity: Severity,
    pub category: String,
    pub resource_kind: String,
    pub resource_name: String,
    pub namespace: Option<String>,
    pub detail: String,
    pub remediation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityReport {
    pub context: String,
    pub findings: Vec<Finding>,
    pub scanned_at_ms: i64,
    pub counts_by_severity: std::collections::BTreeMap<String, i32>,
    pub resources_scanned: i32,
}

// ─── Pod Details (Lens-style detail panel) ──────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PodCondition {
    pub r#type: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub ready: bool,
    pub restart_count: i32,
    /// "Running" | "Waiting:reason" | "Terminated:reason"
    pub state: String,
    pub cpu_request_milli: Option<i64>,
    pub cpu_limit_milli: Option<i64>,
    pub mem_request_bytes: Option<i64>,
    pub mem_limit_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PodDetails {
    pub name: String,
    pub namespace: String,
    /// Pod phase.
    pub status: String,
    pub qos_class: String,
    pub node_name: Option<String>,
    pub controlled_by: Option<OwnerRefLite>,
    pub service_account: Option<String>,
    pub pod_ip: Option<String>,
    pub pod_ips: Vec<String>,
    pub conditions: Vec<PodCondition>,
    pub tolerations: i32,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub containers: Vec<ContainerInfo>,
    pub cpu_usage_milli: Option<i64>,
    pub mem_usage_bytes: Option<i64>,
    pub age_seconds: i64,
    pub created_at_ms: i64,
}
