//! Bounded read-only evidence for one UID-pinned pod.
use super::gpu_types::{resource_ref, Explanation, SchedulingSnapshot, Source, SourceState};
use crate::error::{AppError, AppResult};
use kube::{
    api::{DynamicObject, ListParams},
    core::{ApiResource, GroupVersionKind},
    Api, Client,
};
use serde_json::{json, Map, Value};
use std::{collections::HashMap, time::Duration};
const SOURCE_TIMEOUT: Duration = Duration::from_secs(15);
#[derive(Clone, Copy)]
enum ResourceKind {
    Pod,
    Event,
    Node,
    Pvc,
    Claim,
}
pub fn redact_message(message: &str) -> String {
    // Omit Secret YAML maps and whole credential lines before tokenizing inline values.
    let has_secret = message.lines().any(|line| line.trim() == "kind: Secret");
    let mut map_indent = None;
    let mut safe_lines = Vec::new();
    for line in message.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if let Some(start) = map_indent {
            if trimmed.trim().is_empty() || indent > start {
                continue;
            }
            map_indent = None;
        }
        if has_secret && (trimmed.starts_with("data:") || trimmed.starts_with("stringData:")) {
            safe_lines.push(format!(
                "{}{}: [REDACTED]\n",
                " ".repeat(indent),
                trimmed.split(':').next().unwrap_or("data")
            ));
            map_indent = Some(indent);
            continue;
        }
        let key = trimmed
            .split([':', '='])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let credential = [
            "token",
            "password",
            "secret",
            "api_key",
            "api-key",
            "client-key-data",
            "client-certificate-data",
        ]
        .iter()
        .any(|s| key.contains(s));
        if credential && !key.chars().any(char::is_whitespace) {
            if let Some(i) = line.find([':', '=']) {
                safe_lines.push(format!(
                    "{}[REDACTED]{}",
                    &line[..=i],
                    if line.ends_with('\n') { "\n" } else { "" }
                ));
                continue;
            }
        }
        safe_lines.push(line.to_owned());
    }
    let safe = safe_lines.concat();
    // Scheduler statements retain punctuation and wording outside credential values.
    let mut result = String::new();
    let mut redact_next = false;
    for part in safe.split_inclusive(char::is_whitespace) {
        let word = part.trim_end();
        let suffix = &part[word.len()..];
        let lower = word.to_ascii_lowercase();
        if redact_next {
            result.push_str("[REDACTED]");
            result.push_str(suffix);
            redact_next = false;
            continue;
        }
        let secret = lower.contains("token")
            || lower.contains("password")
            || lower.contains("secret")
            || lower.contains("api_key")
            || lower.contains("api-key")
            || lower.contains("client-key-data");
        if secret {
            if let Some(i) = word.find(['=', ':']) {
                result.push_str(&word[..=i]);
                if i + 1 == word.len() {
                    redact_next = true;
                } else {
                    result.push_str("[REDACTED]");
                }
                result.push_str(suffix);
                continue;
            }
        }
        result.push_str(part);
        if lower == "bearer" {
            redact_next = true;
        }
    }
    result
}
fn select(v: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|k| v.get(*k).map(|x| (k.to_string(), x.clone())))
            .collect::<Map<_, _>>(),
    )
}
fn sanitize(v: Value, kind: ResourceKind) -> Value {
    let metadata = select(
        &v["metadata"],
        &["name", "namespace", "uid", "generation", "ownerReferences"],
    );
    let mut out = json!({"apiVersion":v["apiVersion"],"kind":v["kind"],"metadata":metadata});
    match kind {
        ResourceKind::Pod => {
            out["spec"] = select(
                &v["spec"],
                &[
                    "nodeName",
                    "nodeSelector",
                    "affinity",
                    "tolerations",
                    "schedulingGates",
                    "resourceClaims",
                    "overhead",
                ],
            );
            if let Some(vols) = v.pointer("/spec/volumes").and_then(Value::as_array) {
                out["spec"]["volumes"] = json!(vols
                    .iter()
                    .map(|x| select(x, &["name", "persistentVolumeClaim"]))
                    .collect::<Vec<_>>());
            }
            for key in ["containers", "initContainers"] {
                if let Some(cs) = v["spec"][key].as_array() {
                    out["spec"][key] = json!(cs
                        .iter()
                        .map(|c| select(c, &["name", "resources", "restartPolicy"]))
                        .collect::<Vec<_>>());
                }
            }
            out["status"] = select(
                &v["status"],
                &["phase", "conditions", "resourceClaimStatuses"],
            );
            for key in ["containerStatuses", "initContainerStatuses"] {
                if let Some(cs) = v["status"][key].as_array() {
                    out["status"][key] = json!(cs
                        .iter()
                        .map(|c| select(c, &["name", "state"]))
                        .collect::<Vec<_>>());
                }
            }
        }
        ResourceKind::Event => {
            for k in [
                "involvedObject",
                "reason",
                "message",
                "lastTimestamp",
                "eventTime",
                "firstTimestamp",
                "type",
            ] {
                if let Some(x) = v.get(k) {
                    out[k] = x.clone();
                }
            }
        }
        ResourceKind::Node => {
            out["metadata"]["labels"] = v["metadata"]["labels"].clone();
            out["spec"] = select(&v["spec"], &["taints", "unschedulable"]);
            out["status"] = select(&v["status"], &["allocatable", "conditions"]);
        }
        ResourceKind::Pvc => {
            out["spec"] = select(
                &v["spec"],
                &["storageClassName", "volumeName", "accessModes"],
            );
            out["status"] = select(&v["status"], &["phase", "conditions"]);
        }
        ResourceKind::Claim => {
            out["status"] = select(&v["status"], &["allocation", "reservedFor", "conditions"]);
            if let Some(a) = out.pointer_mut("/status/allocation") {
                *a = select(a, &["nodeSelector"]);
            }
        }
    }
    fn messages(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    if k == "message" {
                        if let Some(s) = v.as_str() {
                            *v = json!(redact_message(s));
                        }
                    } else {
                        messages(v);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    messages(v);
                }
            }
            _ => {}
        }
    }
    messages(&mut out);
    out
}
fn api(client: &Client, ns: &str, group: &str, kind: &str, plural: &str) -> Api<DynamicObject> {
    let ar = ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(group, "v1", kind), plural);
    if ns.is_empty() {
        Api::all_with(client.clone(), &ar)
    } else {
        Api::namespaced_with(client.clone(), ns, &ar)
    }
}
fn source() -> Source<Value> {
    Source {
        state: SourceState::Available,
        complete: true,
        captured_at: chrono::Utc::now().to_rfc3339(),
        items: vec![],
        message: None,
    }
}
fn failure(e: kube::Error) -> Source<Value> {
    let mut s = source();
    s.complete = false;
    match e {
        kube::Error::Api(e) if e.code == 403 => {
            s.state = SourceState::Forbidden;
            s.message = Some("Permission to read this source was denied.".into());
        }
        kube::Error::Api(e) if e.code == 404 => {
            s.state = SourceState::Unsupported;
            s.message = Some("Referenced resource or API endpoint is unavailable.".into());
        }
        _ => {
            s.state = SourceState::Error;
            s.message = Some("Kubernetes source request failed.".into());
        }
    }
    s
}
fn timeout_source() -> Source<Value> {
    let mut s = source();
    s.state = SourceState::Error;
    s.complete = false;
    s.message = Some("Source request timed out; evidence is incomplete.".into());
    s
}
async fn list_source(
    api: &Api<DynamicObject>,
    kind: ResourceKind,
    uid: Option<&str>,
    pages: usize,
    deadline: Duration,
) -> Source<Value> {
    tokio::time::timeout(deadline, async {
        let mut s = source();
        let mut token = String::new();
        let cap = 500 * pages;
        for _ in 0..pages {
            let mut lp = ListParams::default().limit(500);
            if let Some(uid) = uid {
                lp = lp.fields(&format!("involvedObject.uid={uid}"));
            }
            if !token.is_empty() {
                lp = lp.continue_token(&token);
            }
            let page = match api.list(&lp).await {
                Ok(p) => p,
                Err(e) => {
                    let mut err = failure(e);
                    err.items = s.items;
                    return err;
                }
            };
            let overflow = page.items.len() > cap.saturating_sub(s.items.len());
            for v in page
                .items
                .into_iter()
                .take(cap.saturating_sub(s.items.len()))
            {
                let v = serde_json::to_value(v).unwrap_or(Value::Null);
                if uid.is_none() || v.pointer("/involvedObject/uid").and_then(Value::as_str) == uid
                {
                    s.items.push(sanitize(v, kind));
                }
            }
            token = page.metadata.continue_.unwrap_or_default();
            if token.is_empty() && !overflow {
                return s;
            }
            if overflow || s.items.len() >= cap {
                break;
            }
        }
        s.complete = false;
        s.message = Some("Collection truncated at the source request limit.".into());
        s
    })
    .await
    .unwrap_or_else(|_| timeout_source())
}
async fn get_pod(api: &Api<DynamicObject>, name: &str, uid: &str) -> AppResult<Value> {
    let pod = tokio::time::timeout(SOURCE_TIMEOUT, api.get(name))
        .await
        .map_err(|_| AppError::K8s("Selected pod read timed out.".into()))?
        .map_err(|_| AppError::K8s("Selected pod could not be read.".into()))?;
    if pod.metadata.uid.as_deref() != Some(uid) {
        return Err(AppError::Conflict(
            "Selected pod was replaced; select the current pod again.".into(),
        ));
    }
    Ok(sanitize(
        serde_json::to_value(pod)
            .map_err(|_| AppError::Internal("Pod projection failed.".into()))?,
        ResourceKind::Pod,
    ))
}
async fn referenced(client: &Client, ns: &str, pod: &Value, claims: bool) -> Source<Value> {
    tokio::time::timeout(SOURCE_TIMEOUT, async {
        let mut s = source();
        let mut names = Vec::new();
        if claims {
            if let Some(cs) = pod
                .pointer("/spec/resourceClaims")
                .and_then(Value::as_array)
            {
                for c in cs {
                    let direct = c["resourceClaimName"].as_str();
                    let generated = pod
                        .pointer("/status/resourceClaimStatuses")
                        .and_then(Value::as_array)
                        .and_then(|a| a.iter().find(|x| x["name"] == c["name"]))
                        .and_then(|c| c["resourceClaimName"].as_str());
                    if let Some(n) = direct.or(generated) {
                        names.push(n.to_owned());
                    } else {
                        s.complete = false;
                        s.message =
                            Some("Referenced claim has not resolved to a ResourceClaim.".into());
                    }
                }
            }
        } else if let Some(vs) = pod.pointer("/spec/volumes").and_then(Value::as_array) {
            for v in vs {
                if let Some(n) = v
                    .pointer("/persistentVolumeClaim/claimName")
                    .and_then(Value::as_str)
                {
                    names.push(n.to_owned());
                }
            }
        }
        names.sort();
        names.dedup();
        let (group, kind, plural, rk) = if claims {
            (
                "resource.k8s.io",
                "ResourceClaim",
                "resourceclaims",
                ResourceKind::Claim,
            )
        } else {
            (
                "",
                "PersistentVolumeClaim",
                "persistentvolumeclaims",
                ResourceKind::Pvc,
            )
        };
        let api = api(client, ns, group, kind, plural);
        for name in names.into_iter().take(10000) {
            match api.get(&name).await {
                Ok(o) => s
                    .items
                    .push(sanitize(serde_json::to_value(o).unwrap_or(Value::Null), rk)),
                Err(e) => {
                    let err = failure(e);
                    s.state = err.state;
                    s.complete = false;
                    s.message = err.message;
                }
            }
        }
        s
    })
    .await
    .unwrap_or_else(|_| timeout_source())
}
fn explanation(stage: &str, confidence: &str, message: String, v: &Value, at: &str) -> Explanation {
    Explanation {
        stage: stage.into(),
        confidence: confidence.into(),
        message: redact_message(&message),
        sources: vec![resource_ref(v)],
        captured_at: at.into(),
        transition_time: None,
        observed_generation: None,
    }
}
pub fn explain_pod(pod: &Value, events: &[Value], captured_at: &str) -> Vec<Explanation> {
    let mut out = vec![];
    let uid = pod.pointer("/metadata/uid").and_then(Value::as_str);
    for event in events.iter().filter(|e| {
        uid.is_some() && e.pointer("/involvedObject/uid").and_then(Value::as_str) == uid
    }) {
        if event["reason"] == "FailedScheduling" {
            let mut e = explanation(
                "scheduling",
                "observed",
                event["message"]
                    .as_str()
                    .unwrap_or("Scheduler reported a scheduling failure.")
                    .into(),
                event,
                captured_at,
            );
            e.transition_time = event["lastTimestamp"]
                .as_str()
                .or(event["eventTime"].as_str())
                .map(str::to_owned);
            out.push(e);
        }
    }
    if let Some(cs) = pod.pointer("/status/conditions").and_then(Value::as_array) {
        for c in cs {
            if c["type"] == "PodScheduled" && c["status"] == "False" {
                let mut e = explanation(
                    "scheduling",
                    "observed",
                    c["message"]
                        .as_str()
                        .unwrap_or("Pod is not scheduled.")
                        .into(),
                    pod,
                    captured_at,
                );
                e.transition_time = c["lastTransitionTime"].as_str().map(str::to_owned);
                e.observed_generation = c["observedGeneration"].as_i64();
                if let (Some(observed), Some(generation)) = (
                    e.observed_generation,
                    pod.pointer("/metadata/generation").and_then(Value::as_i64),
                ) {
                    if observed < generation {
                        e.confidence = "unknown".into();
                        e.message = format!("Stale condition: {}", e.message);
                    }
                }
                out.push(e);
            }
        }
    }
    if pod
        .pointer("/spec/nodeName")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
    {
        for key in ["initContainerStatuses", "containerStatuses"] {
            if let Some(cs) = pod["status"][key].as_array() {
                for c in cs {
                    if let Some(w) = c.pointer("/state/waiting") {
                        out.push(explanation(
                            "startup",
                            "observed",
                            format!(
                                "{}: {}. {}",
                                c["name"].as_str().unwrap_or("container"),
                                w["reason"].as_str().unwrap_or("Waiting"),
                                w["message"].as_str().unwrap_or("")
                            ),
                            pod,
                            captured_at,
                        ));
                    }
                }
            }
        }
    }
    if pod
        .pointer("/spec/nodeSelector")
        .is_some_and(|s| s.as_object().is_some_and(|m| !m.is_empty()))
        || pod.pointer("/spec/affinity").is_some()
    {
        out.push(explanation("scheduling","inferred","Pod configuration restricts placement through node selectors or affinity; scheduler eligibility is not simulated.".into(),pod,captured_at));
    }
    out
}
pub async fn snapshot(
    client: &Client,
    namespace: &str,
    pod: &str,
    expected_uid: &str,
) -> AppResult<SchedulingSnapshot> {
    let pods = api(client, namespace, "", "Pod", "pods");
    let p = get_pod(&pods, pod, expected_uid).await?;
    let events_api = api(client, namespace, "", "Event", "events");
    let nodes_api = api(client, "", "", "Node", "nodes");
    let (events, nodes, pvcs, claims) = tokio::join!(
        list_source(
            &events_api,
            ResourceKind::Event,
            Some(expected_uid),
            20,
            SOURCE_TIMEOUT
        ),
        list_source(&nodes_api, ResourceKind::Node, None, 20, SOURCE_TIMEOUT),
        referenced(client, namespace, &p, false),
        referenced(client, namespace, &p, true)
    );
    get_pod(&pods, pod, expected_uid).await?;
    let at = chrono::Utc::now().to_rfc3339();
    let mut explanations = explain_pod(&p, &events.items, &at);
    for pvc in &pvcs.items {
        if pvc.pointer("/status/phase").and_then(Value::as_str) == Some("Pending") {
            explanations.push(explanation(
                "scheduling",
                "observed",
                "Referenced PVC is Pending; storage binding has not completed.".into(),
                pvc,
                &at,
            ));
        }
    }
    for claim in &claims.items {
        if claim.pointer("/status/allocation").is_none() {
            explanations.push(explanation(
                "scheduling",
                "observed",
                "Referenced claim has no reported allocation.".into(),
                claim,
                &at,
            ));
        }
    }
    let mut pod_source = source();
    pod_source.items.push(p.clone());
    let sources = HashMap::from([
        ("pod".into(), pod_source),
        ("events".into(), events),
        ("nodes".into(), nodes),
        ("pvcs".into(), pvcs),
        ("claims".into(), claims),
    ]);
    for (name, s) in &sources {
        if s.state != SourceState::Available || !s.complete {
            explanations.push(explanation(
                "scheduling",
                "unknown",
                format!(
                    "{name} evidence is incomplete. {}",
                    s.message
                        .as_deref()
                        .unwrap_or("Placement cannot be established.")
                ),
                &p,
                &at,
            ));
        }
    }
    if explanations.is_empty() {
        explanations.push(explanation("scheduling","unknown","No current blocker is established by the collected evidence; this does not establish successful placement.".into(),&p,&at));
    }
    Ok(SchedulingSnapshot {
        pod: resource_ref(&p),
        sources,
        explanations,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_an_event_for_a_replaced_pod() {
        let pod = serde_json::json!({"metadata":{"name":"train","uid":"new"},"status":{"phase":"Pending"}});
        let event = serde_json::json!({"involvedObject":{"uid":"old"},"reason":"FailedScheduling","message":"Insufficient nvidia.com/gpu"});
        let evidence = explain_pod(&pod, &[event], "2026-10-02T00:00:00Z");
        assert!(!evidence.iter().any(|e| e.message.contains("Insufficient")));
    }
    #[test]
    fn preserves_multiple_scheduler_reasons_and_startup_stage() {
        let pod = serde_json::json!({"metadata":{"name":"train","uid":"u"},"spec":{"nodeName":"node"},"status":{"phase":"Pending","initContainerStatuses":[{"name":"download","state":{"waiting":{"reason":"ImagePullBackOff","message":"retry"}}}]}});
        let event = serde_json::json!({"metadata":{"name":"ev","uid":"ev"},"involvedObject":{"uid":"u"},"reason":"FailedScheduling","message":"Insufficient nvidia.com/gpu, untolerated taint"});
        let evidence = explain_pod(&pod, &[event], "now");
        assert!(evidence
            .iter()
            .any(|e| e.message == "Insufficient nvidia.com/gpu, untolerated taint"));
        assert!(evidence
            .iter()
            .any(|e| e.stage == "startup" && e.message.contains("download")));
    }
}
#[cfg(test)]
mod reader_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        matchers::{method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    fn client(s: &MockServer) -> Client {
        Client::try_from(kube::Config::new(s.uri().parse().unwrap())).unwrap()
    }
    fn pod() -> Value {
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"train","namespace":"ns","uid":"u"},"spec":{"containers":[{"name":"c","env":[{"name":"TOKEN","value":"top-secret"}]}],"volumes":[{"persistentVolumeClaim":{"claimName":"data"}}],"resourceClaims":[{"name":"gpu","resourceClaimName":"claim"}]},"status":{"phase":"Pending"}})
    }
    async fn response(s: &MockServer, p: &str, code: u16, v: Value) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(code).set_body_json(v))
            .mount(s)
            .await;
    }
    fn list(v: Value, next: &str) -> Value {
        json!({"apiVersion":"v1","kind":"List","metadata":{"continue":next},"items":v})
    }
    fn denied() -> Value {
        json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"Forbidden","code":403,"message":"upstream-secret"})
    }
    async fn setup(s: &MockServer) {
        response(s, "/api/v1/namespaces/ns/pods/train", 200, pod()).await;
        response(s, "/api/v1/namespaces/ns/events", 200, list(json!([]), "")).await;
        response(s, "/api/v1/nodes", 200, list(json!([]), "")).await;
        response(s,"/api/v1/namespaces/ns/persistentvolumeclaims/data",200,json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":"data","uid":"pvc"},"status":{"phase":"Pending"}})).await;
        response(s,"/apis/resource.k8s.io/v1/namespaces/ns/resourceclaims/claim",200,json!({"apiVersion":"resource.k8s.io/v1","kind":"ResourceClaim","metadata":{"name":"claim","uid":"claim"},"spec":{"devices":{"config":[{"secret":"driver-secret"}]}},"status":{}})).await;
    }
    #[tokio::test]
    async fn independent_denials_preserve_pvc_claims_and_safe_projection() {
        let s = MockServer::start().await;
        setup(&s).await;
        for p in ["/api/v1/namespaces/ns/events", "/api/v1/nodes"] {
            Mock::given(path(p))
                .respond_with(ResponseTemplate::new(403).set_body_json(denied()))
                .with_priority(1)
                .mount(&s)
                .await;
        }
        let out = snapshot(&client(&s), "ns", "train", "u").await.unwrap();
        assert_eq!(out.sources["events"].state, SourceState::Forbidden);
        assert_eq!(out.sources["nodes"].state, SourceState::Forbidden);
        assert!(out.explanations.iter().any(|e| e.message.contains("PVC")));
        assert!(out.explanations.iter().any(|e| e.message.contains("claim")));
        let text = serde_json::to_string(&out).unwrap();
        for secret in ["top-secret", "upstream-secret", "driver-secret"] {
            assert!(!text.contains(secret));
        }
    }
    #[tokio::test]
    async fn rejects_replacement_at_final_read() {
        let s = MockServer::start().await;
        setup(&s).await;
        Mock::given(path("/api/v1/namespaces/ns/pods/train"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pod()))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&s)
            .await;
        let mut replacement = pod();
        replacement["metadata"]["uid"] = json!("new");
        Mock::given(path("/api/v1/namespaces/ns/pods/train"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replacement))
            .with_priority(2)
            .mount(&s)
            .await;
        assert!(matches!(
            snapshot(&client(&s), "ns", "train", "u").await,
            Err(AppError::Conflict(_))
        ));
    }
    #[tokio::test]
    async fn pages_and_truncation_are_distinct() {
        let s = MockServer::start().await;
        Mock::given(path("/api/v1/nodes"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(list(json!([{"metadata":{"name":"n"}}]), "next")),
            )
            .with_priority(2)
            .mount(&s)
            .await;
        Mock::given(path("/api/v1/nodes"))
            .and(query_param("continue", "next"))
            .respond_with(ResponseTemplate::new(200).set_body_json(list(json!([]), "")))
            .with_priority(1)
            .mount(&s)
            .await;
        let api = api(&client(&s), "", "", "Node", "nodes");
        let out = list_source(&api, ResourceKind::Node, None, 2, Duration::from_secs(1)).await;
        assert!(out.complete);
        assert_eq!(out.items.len(), 1);
        let limited = list_source(&api, ResourceKind::Node, None, 1, Duration::from_secs(1)).await;
        assert!(!limited.complete);
    }
    #[tokio::test]
    async fn injected_deadline_returns_incomplete_timeout() {
        let s = MockServer::start().await;
        Mock::given(path("/api/v1/nodes"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_json(list(json!([]), "")),
            )
            .mount(&s)
            .await;
        let out = list_source(
            &api(&client(&s), "", "", "Node", "nodes"),
            ResourceKind::Node,
            None,
            20,
            Duration::from_millis(5),
        )
        .await;
        assert!(!out.complete);
        assert!(out.message.unwrap().contains("timed out"));
        assert_eq!(SOURCE_TIMEOUT, Duration::from_secs(15));
    }
    #[tokio::test]
    async fn unresolved_template_claim_is_unknown() {
        let s = MockServer::start().await;
        setup(&s).await;
        let mut p = pod();
        p["spec"]["resourceClaims"] =
            json!([{"name":"generated","resourceClaimTemplateName":"template"}]);
        Mock::given(path("/api/v1/namespaces/ns/pods/train"))
            .respond_with(ResponseTemplate::new(200).set_body_json(p))
            .with_priority(1)
            .mount(&s)
            .await;
        let out = snapshot(&client(&s), "ns", "train", "u").await.unwrap();
        assert!(!out.sources["claims"].complete);
        assert!(out
            .explanations
            .iter()
            .any(|e| e.confidence == "unknown" && e.message.contains("claim")));
    }
    #[test]
    fn redacts_messages_but_keeps_scheduler_constraints() {
        let p = pod();
        let ev = json!({"involvedObject":{"uid":"u"},"reason":"FailedScheduling","message":"Insufficient nvidia.com/gpu; token=private; password: private2"});
        let out = explain_pod(&p, &[ev], "now");
        let text = serde_json::to_string(&out).unwrap();
        assert!(text.contains("Insufficient nvidia.com/gpu"));
        assert!(!text.contains("private"));
    }
}

#[cfg(test)]
mod freshness_tests {
    use super::*;
    #[test]
    fn stale_controller_conditions_are_not_current_observations() {
        let p = json!({"apiVersion":"v1","kind":"Pod","metadata":{"uid":"u","generation":5},"status":{"conditions":[{"type":"PodScheduled","status":"False","message":"stale failure","observedGeneration":4,"lastTransitionTime":"2026-10-01T00:00:00Z"}]}});
        let out = explain_pod(&p, &[], "2026-10-02T00:00:00Z");
        assert_eq!(out[0].confidence, "unknown");
        assert_eq!(out[0].observed_generation, Some(4));
        assert_eq!(
            out[0].transition_time.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
    }
}
#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn native_redaction_covers_secret_maps_and_quoted_assignments() {
        let raw="Insufficient gpu\nkind: Secret\ndata:\n  key: hidden-secret\npassword: \"multi word secret\"\nBearer bearer-secret";
        let out = redact_message(raw);
        for secret in ["hidden-secret", "multi word secret", "bearer-secret"] {
            assert!(!out.contains(secret));
        }
        assert!(out.contains("Insufficient gpu"));
    }
}
