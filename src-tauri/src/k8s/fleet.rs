//! Fleet: parallel, bounded probes across every kubeconfig context.
//!
//! Produces one `FleetCard` per context. A context that fails to connect
//! still produces a card with `reachable = false` and an error string —
//! the UI shows these in an "unreachable" bucket rather than erroring the
//! whole fleet view.

use crate::error::{AppError, AppResult};
use crate::k8s::client::K8sState;
use crate::k8s::kubeconfig;
use crate::k8s::metrics;
use crate::k8s::types::{ContextInfo, FleetCard, FleetHealth, NodeSummary};
use futures::future::join_all;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Namespace, Node, Pod};
use kube::{api::ListParams, Api, Client};
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

const NOT_READY: &str = "Ready";

pub fn node_summary(n: &Node) -> NodeSummary {
    let status = n.status.as_ref();
    let roles: Vec<String> = n
        .metadata
        .labels
        .as_ref()
        .map(|m| {
            m.keys()
                .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/").map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let ready = status
        .and_then(|s| s.conditions.as_ref())
        .map(|cs| {
            cs.iter()
                .any(|c| c.type_ == NOT_READY && c.status == "True")
        })
        .unwrap_or(false);
    let ni = status.and_then(|s| s.node_info.as_ref());
    let version = ni.map(|n| n.kubelet_version.clone()).unwrap_or_default();
    let os_image = ni.map(|n| n.os_image.clone()).unwrap_or_default();
    let arch = ni.map(|n| n.architecture.clone()).unwrap_or_default();
    let capacity = status.and_then(|s| s.capacity.as_ref());
    let allocatable = status.and_then(|s| s.allocatable.as_ref());
    let cpu_cap = capacity
        .and_then(|m| m.get("cpu"))
        .and_then(|q| metrics::parse_cpu_milli(&q.0))
        .unwrap_or(0);
    let mem_cap = capacity
        .and_then(|m| m.get("memory"))
        .and_then(|q| metrics::parse_memory_bytes(&q.0))
        .unwrap_or(0);
    let pods_cap = capacity
        .and_then(|m| m.get("pods"))
        .and_then(|q| q.0.parse::<i64>().ok())
        .unwrap_or(0);
    let cpu_alloc = allocatable
        .and_then(|m| m.get("cpu"))
        .and_then(|q| metrics::parse_cpu_milli(&q.0))
        .unwrap_or(0);
    let mem_alloc = allocatable
        .and_then(|m| m.get("memory"))
        .and_then(|q| metrics::parse_memory_bytes(&q.0))
        .unwrap_or(0);
    let taints = n
        .spec
        .as_ref()
        .and_then(|s| s.taints.as_ref())
        .map(|ts| {
            ts.iter()
                .map(|t| {
                    format!(
                        "{}={}:{}",
                        t.key,
                        t.value.clone().unwrap_or_default(),
                        t.effect
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let unschedulable = n
        .spec
        .as_ref()
        .and_then(|s| s.unschedulable)
        .unwrap_or(false);
    NodeSummary {
        name: n.metadata.name.clone().unwrap_or_default(),
        roles,
        version,
        ready,
        os_image,
        arch,
        cpu_capacity_milli: cpu_cap,
        mem_capacity_bytes: mem_cap,
        pods_capacity: pods_cap,
        cpu_allocatable_milli: cpu_alloc,
        mem_allocatable_bytes: mem_alloc,
        taints,
        age_seconds: crate::k8s::resources::age_seconds(&n.metadata),
        cpu_usage_milli: None,
        mem_usage_bytes: None,
        unschedulable,
    }
}

/// List nodes for a single context, enriched with metrics-server usage where
/// available.
pub async fn list_nodes(client: &Client, ctx: &str) -> AppResult<Vec<NodeSummary>> {
    let api: Api<Node> = Api::all(client.clone());
    let raw = api
        .list(&ListParams::default())
        .await
        .map_err(|e| crate::error::AppError::K8s(e.to_string()))?;
    let mut nodes: Vec<NodeSummary> = raw.items.iter().map(node_summary).collect();
    if let Some(usages) = metrics::node_usage(client, ctx).await {
        for u in usages {
            if let Some(n) = nodes.iter_mut().find(|n| n.name == u.name) {
                n.cpu_usage_milli = Some(u.cpu_milli);
                n.mem_usage_bytes = Some(u.mem_bytes);
            }
        }
    }
    Ok(nodes)
}

/// Bound credential setup and the lightweight connectivity request separately
/// from inventory downloads, which can be large even on reachable clusters.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
const INVENTORY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

async fn inventory_read<T>(
    request: impl std::future::Future<Output = Result<T, kube::Error>>,
) -> AppResult<T> {
    tokio::time::timeout(INVENTORY_TIMEOUT, request)
        .await
        .map_err(|_| AppError::K8s("inventory request timed out".into()))?
        .map_err(|e| AppError::K8s(e.to_string()))
}

/// Cap concurrent context probes so a kubeconfig with 8+ contexts doesn't
/// fan out to 80+ simultaneous kube list calls (each `probe_one_inner` does
/// 9 internal parallel calls). Beyond ~6 outer probes we mostly see
/// head-of-line blocking from a single slow context starving the others;
/// this Semaphore lets healthy contexts complete promptly while stalled ones
/// run out bounded connection and inventory deadlines in the background.
const FLEET_CONCURRENCY: usize = 6;

static FLEET_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(FLEET_CONCURRENCY)));

fn unreachable_card(ctx: ContextInfo, err: String, fetched_at_ms: i64) -> FleetCard {
    FleetCard {
        context: ctx,
        reachable: false,
        error: Some(err),
        server_version: None,
        node_count: 0,
        node_ready: 0,
        namespace_count: 0,
        workload_count: 0,
        health: FleetHealth {
            pods_total: 0,
            pods_ready: 0,
            pods_pending: 0,
            pods_failed: 0,
        },
        cpu_percent: None,
        mem_percent: None,
        fetched_at_ms,
        distribution: None,
    }
}

/// Best-effort cluster-distribution detection.
///
/// Walks node labels and looks for vendor-specific markers — these are stable
/// across cloud-provider node-pool versions and don't require any extra API
/// calls beyond the node list we already fetch for fleet stats. Falls back to
/// kubelet OS image matching for the local-development distros (kind, k3s,
/// minikube) which don't tag nodes with provider labels.
///
/// Returns the lowercase identifier matching what the FleetView UI expects:
/// "eks" / "gke" / "aks" / "openshift" / "k3s" / "kind" / "minikube" /
/// "docker-desktop" / "rancher", or `None` for an unrecognized cluster.
pub fn detect_distribution(nodes: &[Node]) -> Option<String> {
    // Single pass; first match wins. Order is deliberate — vendor-specific
    // labels are checked before generic ones (e.g. an EKS node also reports
    // a "kubernetes.io/hostname" label, which would over-match a generic
    // detector if put earlier).
    for node in nodes {
        let labels = node.metadata.labels.as_ref();
        let provider_id = node.spec.as_ref().and_then(|s| s.provider_id.as_deref());

        if let Some(labels) = labels {
            if labels.keys().any(|k| k.starts_with("eks.amazonaws.com/")) {
                return Some("eks".into());
            }
            if labels
                .keys()
                .any(|k| k.starts_with("cloud.google.com/gke-"))
            {
                return Some("gke".into());
            }
            if labels
                .keys()
                .any(|k| k.starts_with("kubernetes.azure.com/"))
            {
                return Some("aks".into());
            }
            if labels.contains_key("node.openshift.io/os_id") {
                return Some("openshift".into());
            }
            if labels
                .get("node-role.kubernetes.io/master")
                .map(|v| v.as_str())
                == Some("")
                && labels.contains_key("node.kubernetes.io/instance-type")
                && labels
                    .get("node.kubernetes.io/instance-type")
                    .map(|v| v.as_str())
                    == Some("k3s")
            {
                return Some("k3s".into());
            }
        }

        // providerID prefixes are reliable for cloud providers when the
        // label-based detection above missed (older clusters, custom CNI
        // etc.). Cheaper than re-walking labels for variants.
        if let Some(pid) = provider_id {
            if pid.starts_with("aws://") {
                return Some("eks".into());
            }
            if pid.starts_with("gce://") {
                return Some("gke".into());
            }
            if pid.starts_with("azure://") {
                return Some("aks".into());
            }
            if pid.starts_with("kind://") {
                return Some("kind".into());
            }
            if pid.starts_with("k3s://") {
                return Some("k3s".into());
            }
        }

        // OS-image fallback for local distros that don't tag with provider
        // labels. The strings are documented kubelet output across versions.
        if let Some(os) = node
            .status
            .as_ref()
            .and_then(|s| s.node_info.as_ref())
            .map(|n| n.os_image.as_str())
        {
            let os_lower = os.to_lowercase();
            if os_lower.contains("k3s") {
                return Some("k3s".into());
            }
            if os_lower.contains("kind") {
                return Some("kind".into());
            }
            if os_lower.contains("docker desktop") || os_lower.contains("dockerdesktop") {
                return Some("docker-desktop".into());
            }
            if os_lower.contains("minikube") {
                return Some("minikube".into());
            }
            if os_lower.contains("rancheros") || os_lower.contains("rke") {
                return Some("rancher".into());
            }
        }
    }
    None
}

async fn probe_one_inner(state: &K8sState, ctx: ContextInfo) -> FleetCard {
    let fetched_at_ms = chrono::Utc::now().timestamp_millis();
    let connection = tokio::time::timeout(PROBE_TIMEOUT, async {
        let client = state.client_for(&ctx.name).await?;
        let version = client
            .apiserver_version()
            .await
            .map_err(|e| AppError::K8s(e.to_string()))?;
        Ok::<_, AppError>((client, version))
    })
    .await;
    let (client, version) = match connection {
        Ok(Ok(connected)) => connected,
        result => {
            state.invalidate(&ctx.name).await;
            let message = match result {
                Ok(Err(error)) => error.to_string(),
                Err(_) => format!("connection timed out after {}s", PROBE_TIMEOUT.as_secs()),
                Ok(Ok(_)) => unreachable!(),
            };
            return unreachable_card(ctx, message, fetched_at_ms);
        }
    };
    let server_version = Some(format!("v{}.{}", version.major, version.minor));

    let c0 = client.clone();
    let c1 = client.clone();
    let c2 = client.clone();
    let c3 = client.clone();
    let c4 = client.clone();
    let c5 = client.clone();
    let c6 = client.clone();
    let c7 = client.clone();
    let c8 = client.clone();
    let ctx_name = ctx.name.clone();

    let (nodes, ns, pods, dep, ss, ds, cj, jobs, node_metrics) = tokio::join!(
        inventory_read(async move { Api::<Node>::all(c0).list(&ListParams::default()).await }),
        inventory_read(async move { Api::<Namespace>::all(c1).list(&ListParams::default()).await }),
        inventory_read(async move { Api::<Pod>::all(c2).list(&ListParams::default()).await }),
        inventory_read(async move {
            Api::<Deployment>::all(c3)
                .list(&ListParams::default())
                .await
        }),
        inventory_read(async move {
            Api::<StatefulSet>::all(c4)
                .list(&ListParams::default())
                .await
        }),
        inventory_read(async move { Api::<DaemonSet>::all(c5).list(&ListParams::default()).await }),
        inventory_read(async move { Api::<CronJob>::all(c6).list(&ListParams::default()).await }),
        inventory_read(async move { Api::<Job>::all(c7).list(&ListParams::default()).await }),
        async move { metrics::node_usage(&c8, &ctx_name).await },
    );
    // Keep successful reads, but never present missing inventory as a healthy zero.
    // Fixed resource labels avoid exposing provider error bodies in the UI.
    let missing: Vec<_> = [
        ("nodes", nodes.is_err()),
        ("namespaces", ns.is_err()),
        ("pods", pods.is_err()),
        ("deployments", dep.is_err()),
        ("statefulsets", ss.is_err()),
        ("daemonsets", ds.is_err()),
        ("cronjobs", cj.is_err()),
        ("jobs", jobs.is_err()),
    ]
    .into_iter()
    .filter_map(|(name, failed)| failed.then_some(name))
    .collect();
    let error = (!missing.is_empty()).then(|| {
        format!(
            "Inventory incomplete: {}. Open the cluster or reconnect to load missing data.",
            missing.join(", ")
        )
    });

    let (node_count, node_ready, cpu_cap_milli, mem_cap_bytes) = match &nodes {
        Ok(list) => {
            let mut ready = 0i32;
            let mut cpu_cap = 0i64;
            let mut mem_cap = 0i64;
            for n in &list.items {
                let is_ready = n
                    .status
                    .as_ref()
                    .and_then(|s| s.conditions.as_ref())
                    .map(|cs| cs.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
                    .unwrap_or(false);
                if is_ready {
                    ready += 1;
                }
                if let Some(cap) = n.status.as_ref().and_then(|s| s.allocatable.as_ref()) {
                    cpu_cap += cap
                        .get("cpu")
                        .and_then(|q| metrics::parse_cpu_milli(&q.0))
                        .unwrap_or(0);
                    mem_cap += cap
                        .get("memory")
                        .and_then(|q| metrics::parse_memory_bytes(&q.0))
                        .unwrap_or(0);
                }
            }
            (list.items.len() as i32, ready, cpu_cap, mem_cap)
        }
        Err(_) => (0, 0, 0, 0),
    };

    let namespace_count = ns.as_ref().map(|l| l.items.len() as i32).unwrap_or(0);

    let (pods_total, pods_ready, pods_pending, pods_failed) = match &pods {
        Ok(list) => {
            let mut total = 0i32;
            let mut ready = 0i32;
            let mut pending = 0i32;
            let mut failed = 0i32;
            for p in &list.items {
                total += 1;
                let phase = p
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.as_deref())
                    .unwrap_or("");
                match phase {
                    "Running" | "Succeeded" => {
                        let all_ready = p
                            .status
                            .as_ref()
                            .and_then(|s| s.container_statuses.as_ref())
                            .map(|cs| !cs.is_empty() && cs.iter().all(|c| c.ready))
                            .unwrap_or(phase == "Succeeded");
                        if all_ready {
                            ready += 1;
                        } else {
                            pending += 1;
                        }
                    }
                    "Pending" => pending += 1,
                    "Failed" => failed += 1,
                    _ => {}
                }
            }
            (total, ready, pending, failed)
        }
        Err(_) => (0, 0, 0, 0),
    };

    let workload_count = dep.as_ref().map(|l| l.items.len() as i32).unwrap_or(0)
        + ss.as_ref().map(|l| l.items.len() as i32).unwrap_or(0)
        + ds.as_ref().map(|l| l.items.len() as i32).unwrap_or(0)
        + cj.as_ref().map(|l| l.items.len() as i32).unwrap_or(0)
        + jobs.as_ref().map(|l| l.items.len() as i32).unwrap_or(0);

    let (cpu_percent, mem_percent) = match node_metrics {
        Some(usages) if cpu_cap_milli > 0 && mem_cap_bytes > 0 => {
            let cpu_used: i64 = usages.iter().map(|u| u.cpu_milli).sum();
            let mem_used: i64 = usages.iter().map(|u| u.mem_bytes).sum();
            (
                Some((cpu_used as f32 / cpu_cap_milli as f32 * 100.0).clamp(0.0, 100.0)),
                Some((mem_used as f32 / mem_cap_bytes as f32 * 100.0).clamp(0.0, 100.0)),
            )
        }
        _ => (None, None),
    };

    let distribution = nodes
        .as_ref()
        .ok()
        .map(|list| detect_distribution(&list.items))
        .unwrap_or(None);

    FleetCard {
        context: ctx,
        reachable: true,
        error,
        server_version,
        node_count,
        node_ready,
        namespace_count,
        workload_count,
        health: FleetHealth {
            pods_total,
            pods_ready,
            pods_pending,
            pods_failed,
        },
        cpu_percent,
        mem_percent,
        fetched_at_ms,
        distribution,
    }
}

async fn probe_one(state: &K8sState, ctx: ContextInfo) -> FleetCard {
    // Hold one permit for the complete, bounded connection and inventory probe.
    let _permit = FLEET_SEMAPHORE.clone().acquire_owned().await.ok();
    probe_one_inner(state, ctx).await
}

pub async fn probe_all(state: &K8sState) -> AppResult<Vec<FleetCard>> {
    let kc = kubeconfig::load()?;
    let ctxs = kubeconfig::list_contexts_from(&kc);
    let futures = ctxs.into_iter().map(|c| probe_one(state, c));
    let cards = join_all(futures).await;
    Ok(cards)
}

pub async fn probe_context(state: &K8sState, context: &str) -> AppResult<FleetCard> {
    let kc = kubeconfig::load()?;
    let ctx = kubeconfig::list_contexts_from(&kc)
        .into_iter()
        .find(|c| c.name == context)
        .ok_or_else(|| {
            crate::error::AppError::Kubeconfig(format!("context '{}' not in kubeconfig", context))
        })?;
    Ok(probe_one(state, ctx).await)
}

#[cfg(test)]
mod tests {
    use super::detect_distribution;
    use k8s_openapi::api::core::v1::{Node, NodeSpec, NodeStatus, NodeSystemInfo};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn node_with_label(key: &str, value: &str) -> Node {
        let mut labels = BTreeMap::new();
        labels.insert(key.to_string(), value.to_string());
        Node {
            metadata: ObjectMeta {
                labels: Some(labels),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn node_with_provider_id(id: &str) -> Node {
        Node {
            spec: Some(NodeSpec {
                provider_id: Some(id.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn node_with_os_image(os: &str) -> Node {
        Node {
            status: Some(NodeStatus {
                node_info: Some(NodeSystemInfo {
                    architecture: "amd64".into(),
                    boot_id: "".into(),
                    container_runtime_version: "".into(),
                    kernel_version: "".into(),
                    kubelet_version: "".into(),
                    kube_proxy_version: "".into(),
                    machine_id: "".into(),
                    operating_system: "linux".into(),
                    os_image: os.to_string(),
                    system_uuid: "".into(),
                    swap: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn detects_eks_via_node_label() {
        let nodes = vec![node_with_label("eks.amazonaws.com/nodegroup", "default")];
        assert_eq!(detect_distribution(&nodes), Some("eks".into()));
    }

    #[test]
    fn detects_gke_via_node_label() {
        let nodes = vec![node_with_label(
            "cloud.google.com/gke-os-distribution",
            "cos",
        )];
        assert_eq!(detect_distribution(&nodes), Some("gke".into()));
    }

    #[test]
    fn detects_aks_via_node_label() {
        let nodes = vec![node_with_label("kubernetes.azure.com/cluster", "rg-foo")];
        assert_eq!(detect_distribution(&nodes), Some("aks".into()));
    }

    #[test]
    fn detects_openshift_via_node_label() {
        let nodes = vec![node_with_label("node.openshift.io/os_id", "rhcos")];
        assert_eq!(detect_distribution(&nodes), Some("openshift".into()));
    }

    #[test]
    fn detects_kind_via_provider_id() {
        let nodes = vec![node_with_provider_id(
            "kind://docker/lumen-test/lumen-test-control-plane",
        )];
        assert_eq!(detect_distribution(&nodes), Some("kind".into()));
    }

    #[test]
    fn detects_minikube_via_os_image() {
        let nodes = vec![node_with_os_image("Buildroot 2023.02 (minikube)")];
        assert_eq!(detect_distribution(&nodes), Some("minikube".into()));
    }

    #[test]
    fn detects_docker_desktop_via_os_image() {
        let nodes = vec![node_with_os_image("Docker Desktop")];
        assert_eq!(detect_distribution(&nodes), Some("docker-desktop".into()));
    }

    #[test]
    fn returns_none_for_unrecognised_cluster() {
        let nodes = vec![node_with_label("kubernetes.io/hostname", "worker-1")];
        assert_eq!(detect_distribution(&nodes), None);
    }

    #[test]
    fn returns_none_for_empty_node_list() {
        let nodes: Vec<Node> = Vec::new();
        assert_eq!(detect_distribution(&nodes), None);
    }
}
