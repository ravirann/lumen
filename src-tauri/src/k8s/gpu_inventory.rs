use serde_json::Value;
use std::collections::HashMap;

type Requests = HashMap<String, Option<i64>>;

pub fn is_gpu_resource(name: &str) -> bool {
    matches!(name, "nvidia.com/gpu" | "nvidia.com/gpu.shared")
        || name
            .strip_prefix("nvidia.com/mig-")
            .is_some_and(|s| !s.is_empty())
}

pub fn active_pod(pod: &Value) -> bool {
    !matches!(
        pod.pointer("/status/phase").and_then(Value::as_str),
        Some("Succeeded" | "Failed")
    )
}

/// Convert a Kubernetes quantity exactly, without rounding fractional GPU slots.
/// Decimal digit arithmetic avoids overflow in an intermediate mantissa that may
/// still reduce to a small integer (particularly fractional BinarySI quantities).
pub fn gpu_quantity(value: &Value) -> Option<i64> {
    let number;
    let raw = match value {
        Value::String(s) => s.as_str(),
        Value::Number(n) => {
            number = n.to_string();
            &number
        }
        _ => return None,
    };
    let raw = raw.strip_prefix('+').unwrap_or(raw);
    let end = raw
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(raw.len());
    let (mantissa, suffix) = raw.split_at(end);
    let mut parts = mantissa.split('.');
    let whole = parts.next()?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some() || whole.len() + fraction.len() == 0 {
        return None;
    }
    let (decimal_power, binary_power): (i64, u32) = match suffix {
        "" => (0, 0),
        "n" => (-9, 0),
        "u" => (-6, 0),
        "m" => (-3, 0),
        "k" => (3, 0),
        "M" => (6, 0),
        "G" => (9, 0),
        "T" => (12, 0),
        "P" => (15, 0),
        "E" => (18, 0),
        "Ki" => (0, 10),
        "Mi" => (0, 20),
        "Gi" => (0, 30),
        "Ti" => (0, 40),
        "Pi" => (0, 50),
        "Ei" => (0, 60),
        _ if suffix.starts_with(['e', 'E']) => {
            let exponent = &suffix[1..];
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (exponent.parse::<i64>().ok()?, 0)
        }
        _ => return None,
    };
    let joined = format!("{whole}{fraction}");
    let mut digits: Vec<u8> = joined
        .trim_start_matches('0')
        .bytes()
        .map(|b| b - b'0')
        .collect();
    if digits.is_empty() {
        return Some(0);
    }
    for _ in 0..binary_power {
        let mut carry = 0;
        for digit in digits.iter_mut().rev() {
            let doubled = *digit * 2 + carry;
            *digit = doubled % 10;
            carry = doubled / 10;
        }
        if carry != 0 {
            digits.insert(0, carry);
        }
    }
    let scale = decimal_power.checked_sub(i64::try_from(fraction.len()).ok()?)?;
    if scale < 0 {
        let remove = usize::try_from(scale.checked_neg()?).ok()?;
        if remove >= digits.len() || !digits[digits.len() - remove..].iter().all(|d| *d == 0) {
            return None;
        }
        digits.truncate(digits.len() - remove);
    } else {
        let append = usize::try_from(scale).ok()?;
        if digits.len().checked_add(append)? > 19 {
            return None;
        }
        digits.resize(digits.len() + append, 0);
    }
    digits
        .into_iter()
        .try_fold(0i64, |n, d| n.checked_mul(10)?.checked_add(i64::from(d)))
}

fn resource_map(value: &Value) -> Requests {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| is_gpu_resource(key))
        .map(|(key, value)| (key.clone(), gpu_quantity(value)))
        .collect()
}

fn container_requests(container: &Value) -> Requests {
    let mut requests = resource_map(&container["resources"]["requests"]);
    // Admitted API objects ordinarily contain defaulted requests. Handle limit-only
    // objects too, but never substitute a limit for an explicit invalid request.
    for (key, value) in resource_map(&container["resources"]["limits"]) {
        requests.entry(key).or_insert(value);
    }
    requests
}

fn add(target: &mut Requests, other: &Requests) {
    for (key, value) in other {
        let current = target.entry(key.clone()).or_insert(Some(0));
        *current = current.and_then(|a| value.and_then(|b| a.checked_add(b)));
    }
}

fn maximum(target: &mut Requests, other: &Requests) {
    for (key, value) in other {
        let current = target.entry(key.clone()).or_insert(Some(0));
        *current = current.and_then(|a| value.map(|b| a.max(b)));
    }
}

/// Kubernetes v1.35 AggregateContainerRequests / PodRequests stage accounting.
/// Unknown contributions remain unknown even when a known stage is larger.
pub fn effective_gpu_requests(pod: &Value) -> Requests {
    let spec = &pod["spec"];
    let mut steady = Requests::new();
    for container in spec["containers"].as_array().into_iter().flatten() {
        add(&mut steady, &container_requests(container));
    }
    let mut sidecars = Requests::new();
    let mut init_max = Requests::new();
    for container in spec["initContainers"].as_array().into_iter().flatten() {
        let requests = container_requests(container);
        let stage = if container["restartPolicy"].as_str() == Some("Always") {
            add(&mut steady, &requests);
            add(&mut sidecars, &requests);
            sidecars.clone()
        } else {
            let mut stage = sidecars.clone();
            add(&mut stage, &requests);
            stage
        };
        maximum(&mut init_max, &stage);
    }
    maximum(&mut steady, &init_max);
    add(&mut steady, &resource_map(&spec["overhead"]));
    // Extended resources at pod level are outside supported Kubernetes request
    // forms. Preserve the affected key as unknown rather than silently ignoring it.
    for field in ["requests", "limits"] {
        for key in resource_map(&spec["resources"][field]).keys() {
            steady.insert(key.clone(), None);
        }
    }
    steady
}

use super::gpu_types::{resource_ref, GpuNode, GpuPod, ResourceRef, Source, SourceState};
use kube::{
    api::{DynamicObject, ListParams},
    core::{ApiResource, GroupVersionKind},
    Api, Client,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
const INVENTORY_DEADLINE: Duration = Duration::from_secs(15);
const INVENTORY_CAP: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInventorySnapshot {
    pub namespace: String,
    pub captured_at: String,
    pub pods: Source<GpuPod>,
    pub nodes: Source<GpuNode>,
}

pub fn project_gpu_pod(pod: &Value) -> GpuPod {
    let owners = pod
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|owner| ResourceRef {
            api_version: owner["apiVersion"].as_str().unwrap_or_default().into(),
            kind: owner["kind"].as_str().unwrap_or_default().into(),
            name: owner["name"].as_str().unwrap_or_default().into(),
            uid: owner["uid"].as_str().unwrap_or_default().into(),
            namespace: pod
                .pointer("/metadata/namespace")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
        .collect();
    GpuPod {
        resource: resource_ref(pod),
        phase: pod
            .pointer("/status/phase")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into(),
        node_name: pod
            .pointer("/spec/nodeName")
            .and_then(Value::as_str)
            .map(str::to_owned),
        requests: effective_gpu_requests(pod),
        owners,
    }
}
fn allowed_gpu_label(key: &str) -> bool {
    matches!(
        key,
        "nvidia.com/gpu.product"
            | "nvidia.com/mig.strategy"
            | "nvidia.com/mig.config"
            | "nvidia.com/mig.config.state"
            | "nvidia.com/gpu.sharing-strategy"
    ) || key.strip_suffix(".replicas").is_some_and(is_gpu_resource)
}
pub fn project_gpu_node(node: &Value) -> GpuNode {
    GpuNode {
        resource: resource_ref(node),
        allocatable: resource_map(&node["status"]["allocatable"]),
        gpu_labels: node
            .pointer("/metadata/labels")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(key, _)| allowed_gpu_label(key))
            .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_owned())))
            .collect(),
    }
}
fn inventory_api(
    client: &Client,
    namespace: Option<&str>,
    kind: &str,
    plural: &str,
) -> Api<DynamicObject> {
    let resource =
        ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("", "v1", kind), plural);
    match namespace {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &resource),
        None => Api::all_with(client.clone(), &resource),
    }
}
async fn collect<T>(
    api: &Api<DynamicObject>,
    project: fn(&Value) -> T,
    include: impl Fn(&T) -> bool,
    deadline: Duration,
) -> Source<T> {
    let mut source = Source {
        state: SourceState::Available,
        complete: true,
        captured_at: chrono::Utc::now().to_rfc3339(),
        items: Vec::new(),
        message: None,
    };
    let expires = tokio::time::Instant::now() + deadline;
    let mut token = String::new();
    let mut seen = 0usize;
    for _ in 0..20 {
        let mut params = ListParams::default().limit(500);
        if !token.is_empty() {
            params = params.continue_token(&token);
        }
        let page = match tokio::time::timeout_at(expires, api.list(&params)).await {
            Ok(Ok(page)) => page,
            result => {
                source.complete = false;
                let (state, message) = match result {
                    Err(_) => (
                        SourceState::Error,
                        "GPU inventory source timed out; evidence is incomplete.",
                    ),
                    Ok(Err(kube::Error::Api(e))) if e.code == 403 || e.code == 401 => (
                        SourceState::Forbidden,
                        "Permission to read this GPU inventory source was denied.",
                    ),
                    Ok(Err(kube::Error::Api(e))) if e.code == 404 => (
                        SourceState::Unsupported,
                        "This GPU inventory API is unavailable.",
                    ),
                    _ => (SourceState::Error, "GPU inventory source request failed."),
                };
                source.state = state;
                source.message = Some(message.into());
                source.captured_at = chrono::Utc::now().to_rfc3339();
                return source;
            }
        };
        let remaining = INVENTORY_CAP.saturating_sub(seen);
        let overflow = page.items.len() > remaining;
        seen += page.items.len().min(remaining);
        for item in page.items.into_iter().take(remaining) {
            let value =
                serde_json::to_value(item).expect("DynamicObject serialization is infallible");
            let projected = project(&value);
            if include(&projected) {
                source.items.push(projected);
            }
        }
        token = page.metadata.continue_.unwrap_or_default();
        source.captured_at = chrono::Utc::now().to_rfc3339();
        if token.is_empty() && !overflow {
            return source;
        }
        if overflow || seen >= INVENTORY_CAP {
            break;
        }
    }
    source.complete = false;
    source.message = Some("GPU inventory collection truncated at the source request limit.".into());
    source
}
/// Independent source states describe namespace workload requests and cluster-wide
/// advertised slots. These are not physical-device identities or free-GPU totals.
pub async fn snapshot(client: &Client, namespace: &str) -> GpuInventorySnapshot {
    let pods_api = inventory_api(client, Some(namespace), "Pod", "pods");
    let nodes_api = inventory_api(client, None, "Node", "nodes");
    let (pods, nodes) = tokio::join!(
        collect(
            &pods_api,
            project_gpu_pod,
            |p| !p.requests.is_empty(),
            INVENTORY_DEADLINE
        ),
        collect(
            &nodes_api,
            project_gpu_node,
            |n| !n.allocatable.is_empty() || !n.gpu_labels.is_empty(),
            INVENTORY_DEADLINE
        )
    );
    GpuInventorySnapshot {
        namespace: namespace.into(),
        captured_at: chrono::Utc::now().to_rfc3339(),
        pods,
        nodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn container(value: Value) -> Value {
        json!({"resources":{"requests":{"nvidia.com/gpu":value}}})
    }
    fn request(pod: Value) -> Option<Option<i64>> {
        effective_gpu_requests(&pod).get("nvidia.com/gpu").copied()
    }
    #[test]
    fn invalid_quantity_remains_unknown() {
        assert_eq!(
            request(json!({"spec":{"containers":[container(json!("garbage"))]}})),
            Some(None)
        );
    }
    #[test]
    fn completed_jobs_are_not_active_allocations() {
        for phase in ["Succeeded", "Failed"] {
            assert!(!active_pod(&json!({"status":{"phase":phase}})));
        }
        for phase in ["Pending", "Running", "Unknown", ""] {
            assert!(active_pod(&json!({"status":{"phase":phase}})));
        }
    }
    #[test]
    fn recognizes_only_supported_resources() {
        for key in [
            "nvidia.com/gpu",
            "nvidia.com/gpu.shared",
            "nvidia.com/mig-1g.5gb",
        ] {
            assert!(is_gpu_resource(key));
        }
        for key in [
            "cpu",
            "amd.com/gpu",
            "nvidia.com/mig-",
            "nvidia.com/gpu-extra",
        ] {
            assert!(!is_gpu_resource(key));
        }
    }
    #[test]
    fn quantities_use_exact_decimal_binary_and_exponent_semantics() {
        for (value, expected) in [
            ("1", 1),
            ("+2", 2),
            ("1000m", 1),
            ("1000000u", 1),
            ("1000000000n", 1),
            ("1.0", 1),
            (".5Ki", 512),
            ("1.5Ki", 1536),
            ("1e3", 1000),
            ("10E-1", 1),
            ("1k", 1000),
            ("1M", 1000000),
            ("0", 0),
            ("9223372036854775807", i64::MAX),
            (
                "0.000000000000000000867361737988403547205962240695953369140625Ei",
                1,
            ),
        ] {
            assert_eq!(gpu_quantity(&json!(value)), Some(expected), "{value}");
        }
        assert_eq!(gpu_quantity(&json!(2)), Some(2));
    }
    #[test]
    fn rejects_negative_fractional_malformed_overflow_and_wrong_types() {
        for value in [
            "-1",
            "-0",
            "0.1",
            "1m",
            "1.1Ki",
            "9223372036854775808",
            "1EiEi",
            "1K",
            "1e",
            "NaN",
            " 1",
            "1 ",
            ".",
            "1e999999999999999999999999",
            "1e-99999999999999999999999",
        ] {
            assert_eq!(gpu_quantity(&json!(value)), None, "{value}");
        }
        for value in [json!(null), json!(true), json!([]), json!({}), json!(1.5)] {
            assert_eq!(gpu_quantity(&value), None);
        }
    }
    #[test]
    fn sums_apps_and_takes_sequential_init_max() {
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("2")),container(json!("3"))],"initContainers":[container(json!("7")),container(json!("6"))]}})
            ),
            Some(Some(7))
        );
    }
    #[test]
    fn restartable_init_is_added_to_steady_and_only_following_init_stages() {
        let mut sidecar = container(json!("3"));
        sidecar["restartPolicy"] = json!("Always");
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("2"))],"initContainers":[container(json!("8")),sidecar.clone(),container(json!("6"))]}})
            ),
            Some(Some(9))
        );
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("7"))],"initContainers":[sidecar,container(json!("1"))]}})
            ),
            Some(Some(10))
        );
    }
    #[test]
    fn overhead_adds_after_max_including_overhead_only_keys() {
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("2"))],"initContainers":[container(json!("5"))],"overhead":{"nvidia.com/gpu":"1"}}})
            ),
            Some(Some(6))
        );
        assert_eq!(
            request(json!({"spec":{"overhead":{"nvidia.com/gpu":"2"}}})),
            Some(Some(2))
        );
    }
    #[test]
    fn limits_default_only_missing_requests_per_resource() {
        assert_eq!(
            request(
                json!({"spec":{"containers":[{"resources":{"limits":{"nvidia.com/gpu":"3"}}}]}})
            ),
            Some(Some(3))
        );
        assert_eq!(
            request(
                json!({"spec":{"containers":[{"resources":{"requests":{"nvidia.com/gpu":"0"},"limits":{"nvidia.com/gpu":"3"}}}]}})
            ),
            Some(Some(0))
        );
    }
    #[test]
    fn unknown_contributor_and_checked_sum_overflow_remain_unknown() {
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("1"))],"initContainers":[container(json!("garbage")),container(json!("9"))]}})
            ),
            Some(None)
        );
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!(i64::MAX)),container(json!("1"))]}})
            ),
            Some(None)
        );
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!(i64::MAX))],"overhead":{"nvidia.com/gpu":"1"}}})
            ),
            Some(None)
        );
    }
    #[test]
    fn pod_level_gpu_forms_are_unknown_and_unrelated_cpu_does_not_change_gpu() {
        assert_eq!(
            request(json!({"spec":{"resources":{"requests":{"nvidia.com/gpu":"4"}}}})),
            Some(None)
        );
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("2"))],"resources":{"limits":{"nvidia.com/gpu":"4"}}}})
            ),
            Some(None)
        );
        assert_eq!(
            request(
                json!({"spec":{"containers":[container(json!("2"))],"resources":{"requests":{"cpu":"4"}}}})
            ),
            Some(Some(2))
        );
    }
    #[test]
    fn resource_keys_are_independent_and_ephemeral_containers_do_not_reserve() {
        let map = effective_gpu_requests(
            &json!({"spec":{"containers":[{"resources":{"requests":{"nvidia.com/gpu":"bad","nvidia.com/mig-1g.5gb":"2","cpu":"3"}}}],"ephemeralContainers":[container(json!("7"))]}}),
        );
        assert_eq!(map.get("nvidia.com/gpu"), Some(&None));
        assert_eq!(map.get("nvidia.com/mig-1g.5gb"), Some(&Some(2)));
        assert!(!map.contains_key("cpu"));
    }
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    fn client(s: &MockServer) -> kube::Client {
        kube::Client::try_from(kube::Config::new(s.uri().parse().unwrap())).unwrap()
    }
    fn pod(name: &str) -> Value {
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"namespace":"team","uid":name,"annotations":{"secret":"private-marker"}},"spec":{"containers":[{"name":"worker","env":[{"value":"private-marker"}],"resources":{"limits":{"nvidia.com/gpu":"1"}}}]},"status":{"phase":"Pending"}})
    }
    fn node(name: &str) -> Value {
        json!({"apiVersion":"v1","kind":"Node","metadata":{"name":name,"uid":name,"labels":{"nvidia.com/gpu.product":"A100","nvidia.com/gpu.replicas":"8","nvidia.com/mig.config":"all-1g.5gb","private":"private-marker","nvidia.com/arbitrary":"private-marker"}},"status":{"allocatable":{"nvidia.com/gpu.shared":"8","cpu":"4"}}})
    }
    fn list(items: Vec<Value>, next: &str) -> Value {
        json!({"apiVersion":"v1","kind":"List","metadata":{"continue":next},"items":items})
    }
    async fn response(s: &MockServer, p: &str, code: u16, v: Value) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(code).set_body_json(v))
            .mount(s)
            .await;
    }
    #[test]
    fn projection_excludes_environment_values_and_non_allowlisted_metadata() {
        let p = project_gpu_pod(&pod("train"));
        assert_eq!(p.requests["nvidia.com/gpu"], Some(1));
        let n = project_gpu_node(&node("worker"));
        assert_eq!(n.gpu_labels.len(), 3);
        assert_eq!(n.allocatable.len(), 1);
        for serialized in [
            serde_json::to_string(&p).unwrap(),
            serde_json::to_string(&n).unwrap(),
        ] {
            assert!(!serialized.contains("private-marker"));
            assert!(!serialized.contains("annotations"));
        }
    }
    #[test]
    fn projection_preserves_unknown_quantities_terminal_phases_and_owner_identity() {
        let mut value = pod("train");
        value["spec"]["containers"][0]["resources"]["limits"]["nvidia.com/gpu"] = json!("bad");
        value["spec"]["nodeName"] = json!("worker");
        value["status"]["phase"] = json!("Succeeded");
        value["metadata"]["ownerReferences"] = json!([{"apiVersion":"batch/v1","kind":"Job","name":"job","uid":"j1","secret":"private-marker"}]);
        let projected = project_gpu_pod(&value);
        assert_eq!(projected.requests["nvidia.com/gpu"], None);
        assert_eq!(projected.phase, "Succeeded");
        assert_eq!(projected.node_name.as_deref(), Some("worker"));
        assert_eq!(projected.owners[0].uid, "j1");
        assert_eq!(projected.owners[0].namespace.as_deref(), Some("team"));
        assert!(!serde_json::to_string(&projected)
            .unwrap()
            .contains("private-marker"));
        let mut n = node("worker");
        n["status"]["allocatable"]["nvidia.com/gpu.shared"] = json!("0.5");
        assert_eq!(
            project_gpu_node(&n).allocatable["nvidia.com/gpu.shared"],
            None
        );
    }
    #[tokio::test]
    async fn later_permission_failure_retains_successful_page() {
        let s = MockServer::start().await;
        let p = "/api/v1/namespaces/team/pods";
        response(&s, p, 200, list(vec![pod("one")], "next")).await;
        Mock::given(method("GET")).and(path(p)).and(query_param("continue","next"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"kind":"Status","apiVersion":"v1","code":403,"reason":"Forbidden","message":"private-marker"}))).with_priority(1).mount(&s).await;
        response(&s, "/api/v1/nodes", 200, list(vec![], "")).await;
        let out = snapshot(&client(&s), "team").await;
        assert_eq!(out.pods.items.len(), 1);
        assert_eq!(out.pods.state, SourceState::Forbidden);
        assert!(!out.pods.complete);
        assert!(out.nodes.complete);
        assert!(!serde_json::to_string(&out)
            .unwrap()
            .contains("private-marker"));
    }
    #[tokio::test]
    async fn namespace_pods_survive_node_denial_and_errors_are_sanitized() {
        let s = MockServer::start().await;
        response(
            &s,
            "/api/v1/namespaces/team/pods",
            200,
            list(vec![pod("train")], ""),
        )
        .await;
        response(&s,"/api/v1/nodes",403,json!({"kind":"Status","apiVersion":"v1","reason":"Forbidden","code":403,"message":"private-marker"})).await;
        let out = snapshot(&client(&s), "team").await;
        assert_eq!(out.namespace, "team");
        assert_eq!(out.pods.items.len(), 1);
        assert!(out.pods.complete);
        assert_eq!(out.nodes.state, SourceState::Forbidden);
        assert!(!out.nodes.complete);
        assert!(!serde_json::to_string(&out)
            .unwrap()
            .contains("private-marker"));
        assert!(s
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() != "/api/v1/pods"));
    }
    #[tokio::test]
    async fn independent_sources_paginate_and_empty_is_available() {
        let s = MockServer::start().await;
        for (p, a, b) in [
            ("/api/v1/namespaces/team/pods", pod("one"), pod("two")),
            ("/api/v1/nodes", node("one"), node("two")),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .and(query_param("limit", "500"))
                .respond_with(ResponseTemplate::new(200).set_body_json(list(vec![a], "next")))
                .expect(1)
                .mount(&s)
                .await;
            Mock::given(method("GET"))
                .and(path(p))
                .and(query_param("continue", "next"))
                .respond_with(ResponseTemplate::new(200).set_body_json(list(vec![b], "")))
                .with_priority(1)
                .expect(1)
                .mount(&s)
                .await;
        }
        let out = snapshot(&client(&s), "team").await;
        assert!(out.pods.complete && out.nodes.complete);
        assert_eq!(out.pods.items.len(), 2);
        assert_eq!(out.nodes.items.len(), 2);
        s.verify().await;
        let empty = MockServer::start().await;
        for p in ["/api/v1/namespaces/team/pods", "/api/v1/nodes"] {
            response(&empty, p, 200, list(vec![], "")).await;
        }
        let out = snapshot(&client(&empty), "team").await;
        assert_eq!(out.pods.state, SourceState::Available);
        assert!(out.pods.complete && out.pods.items.is_empty());
        assert!(out.nodes.complete && out.nodes.items.is_empty());
    }
    #[tokio::test]
    async fn truncation_counts_all_objects_not_just_gpu_objects() {
        let s = MockServer::start().await;
        let items = (0..500).map(|i| if i==0 {pod("gpu")} else {json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":format!("cpu-{i}"),"namespace":"team"},"spec":{"containers":[]}})}).collect();
        Mock::given(method("GET"))
            .and(path("/api/v1/namespaces/team/pods"))
            .respond_with(ResponseTemplate::new(200).set_body_json(list(items, "more")))
            .expect(20)
            .mount(&s)
            .await;
        response(&s, "/api/v1/nodes", 200, list(vec![], "")).await;
        let out = snapshot(&client(&s), "team").await;
        assert!(!out.pods.complete);
        assert_eq!(out.pods.state, SourceState::Available);
        assert_eq!(out.pods.items.len(), 20);
        assert!(out.nodes.complete);
        s.verify().await;
    }
    #[tokio::test]
    async fn deadline_preserves_prior_pages_and_marks_incomplete() {
        let s = MockServer::start().await;
        let p = "/api/v1/namespaces/team/pods";
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(list(vec![pod("one")], "next")))
            .mount(&s)
            .await;
        Mock::given(method("GET"))
            .and(path(p))
            .and(query_param("continue", "next"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(300))
                    .set_body_json(list(vec![pod("two")], "")),
            )
            .with_priority(1)
            .mount(&s)
            .await;
        let api = inventory_api(&client(&s), Some("team"), "Pod", "pods");
        let out = collect(
            &api,
            project_gpu_pod,
            |p| !p.requests.is_empty(),
            std::time::Duration::from_millis(100),
        )
        .await;
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.state, SourceState::Error);
        assert!(!out.complete);
    }
}
