//! Optional, bounded Kueue admission evidence. Never predicts scheduler outcomes.
use super::gpu_scheduling::redact_message;
use super::gpu_types::{resource_ref, Explanation, ResourceRef, Source, SourceState};
use kube::{
    api::{DynamicObject, ListParams},
    core::{ApiResource, GroupVersionKind},
    Api, Client,
};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};
const GROUP: &str = "kueue.x-k8s.io";
fn source(at: &str) -> Source<Explanation> {
    Source {
        state: SourceState::Available,
        complete: true,
        captured_at: at.into(),
        items: vec![],
        message: None,
    }
}
fn unknown(s: &mut Source<Explanation>, message: &str) {
    s.complete = false;
    s.message = Some(message.into());
}
fn failure(s: &mut Source<Explanation>, e: kube::Error, endpoint: bool) {
    s.state = match e {
        kube::Error::Api(e) if e.code == 403 => SourceState::Forbidden,
        kube::Error::Api(e) if e.code == 404 && endpoint => SourceState::Unsupported,
        _ => SourceState::Error,
    };
    unknown(s, "Optional Kueue evidence could not be read.");
}
fn api(
    client: &Client,
    ns: &str,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
) -> Api<DynamicObject> {
    let ar =
        ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(group, version, kind), plural);
    if ns.is_empty() {
        Api::all_with(client.clone(), &ar)
    } else {
        Api::namespaced_with(client.clone(), ns, &ar)
    }
}
fn controller(v: &Value) -> Result<Option<&Value>, ()> {
    let refs = v
        .pointer("/metadata/ownerReferences")
        .and_then(Value::as_array);
    let refs: Vec<_> = refs
        .into_iter()
        .flatten()
        .filter(|r| r["controller"] == true)
        .collect();
    match refs.len() {
        0 => Ok(None),
        1 => Ok(Some(refs[0])),
        _ => Err(()),
    }
}
fn supported(r: &Value) -> Option<(&str, &str, &str)> {
    match (r["apiVersion"].as_str()?, r["kind"].as_str()?) {
        ("batch/v1", "Job") => Some(("batch", "v1", "jobs")),
        ("batch/v1", "CronJob") => Some(("batch", "v1", "cronjobs")),
        ("apps/v1", "ReplicaSet") => Some(("apps", "v1", "replicasets")),
        ("apps/v1", "Deployment") => Some(("apps", "v1", "deployments")),
        ("apps/v1", "StatefulSet") => Some(("apps", "v1", "statefulsets")),
        ("apps/v1", "DaemonSet") => Some(("apps", "v1", "daemonsets")),
        _ => None,
    }
}
enum OwnershipError {
    Relationship,
    Read(kube::Error),
}
async fn owners(
    client: &Client,
    ns: &str,
    pod: &Value,
) -> Result<Vec<ResourceRef>, OwnershipError> {
    let mut current = pod.clone();
    let mut seen = HashSet::new();
    seen.insert(
        pod.pointer("/metadata/uid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    );
    let mut out = vec![];
    for _ in 0..4 {
        let Some(r) = controller(&current).map_err(|_| OwnershipError::Relationship)? else {
            return Ok(out);
        };
        let (g, v, p) = supported(r).ok_or(OwnershipError::Relationship)?;
        let name = r["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(OwnershipError::Relationship)?;
        let uid = r["uid"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(OwnershipError::Relationship)?;
        if !seen.insert(uid.to_owned()) {
            return Err(OwnershipError::Relationship);
        }
        let object = api(
            client,
            ns,
            g,
            v,
            r["kind"].as_str().ok_or(OwnershipError::Relationship)?,
            p,
        )
        .get(name)
        .await
        .map_err(OwnershipError::Read)?;
        let object = serde_json::to_value(object).map_err(|_| OwnershipError::Relationship)?;
        if object.pointer("/metadata/uid").and_then(Value::as_str) != Some(uid)
            || object
                .pointer("/metadata/namespace")
                .and_then(Value::as_str)
                != Some(ns)
            || object["kind"] != r["kind"]
            || object["apiVersion"] != r["apiVersion"]
        {
            return Err(OwnershipError::Relationship);
        }
        out.push(resource_ref(&object));
        current = object;
    }
    if controller(&current)
        .map_err(|_| OwnershipError::Relationship)?
        .is_some()
    {
        Err(OwnershipError::Relationship)
    } else {
        Ok(out)
    }
}
pub fn matching_workload<'a>(workloads: &'a [Value], owners: &[ResourceRef]) -> Option<&'a Value> {
    let matches: Vec<_> = workloads
        .iter()
        .filter(|w| {
            controller(w).ok().flatten().is_some_and(|r| {
                owners.iter().any(|o| {
                    !o.uid.is_empty()
                        && w.pointer("/metadata/namespace").and_then(Value::as_str)
                            == o.namespace.as_deref()
                        && r["uid"].as_str() == Some(&o.uid)
                        && r["name"].as_str() == Some(&o.name)
                        && r["kind"].as_str() == Some(&o.kind)
                        && r["apiVersion"].as_str() == Some(&o.api_version)
                })
            })
        })
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}
fn explanation(v: &Value, message: String, confidence: &str, at: &str) -> Explanation {
    Explanation {
        stage: "admission".into(),
        confidence: confidence.into(),
        message: redact_message(&message),
        sources: vec![resource_ref(v)],
        captured_at: at.into(),
        transition_time: None,
        observed_generation: None,
    }
}
fn conditions(v: &Value, at: &str) -> Vec<Explanation> {
    let mut out = vec![];
    for c in v
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(kind) = c["type"].as_str() else {
            continue;
        };
        if !matches!(
            kind,
            "QuotaReserved" | "Admitted" | "Evicted" | "Preempted" | "Active" | "Ready"
        ) {
            continue;
        }
        let Some(status) = c["status"]
            .as_str()
            .filter(|s| matches!(*s, "True" | "False" | "Unknown"))
        else {
            continue;
        };
        let observed = c["observedGeneration"].as_i64();
        let generation = v.pointer("/metadata/generation").and_then(Value::as_i64);
        let current = observed.zip(generation).is_some_and(|(o, g)| o >= g);
        let mut e = explanation(
            v,
            format!(
                "{} {kind}={status}: {}",
                v["kind"].as_str().unwrap_or("Kueue"),
                c["message"]
                    .as_str()
                    .unwrap_or("Controller condition reported.")
            ),
            if current && status != "Unknown" {
                "observed"
            } else {
                "unknown"
            },
            at,
        );
        if !current {
            e.message = format!("Condition freshness is unknown: {}", e.message)
        }
        e.transition_time = c["lastTransitionTime"].as_str().map(str::to_owned);
        e.observed_generation = observed;
        out.push(e);
    }
    if v["kind"] == "Workload" {
        for check in v
            .pointer("/status/admissionChecks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(name), Some(state)) = (check["name"].as_str(), check["state"].as_str()) {
                if matches!(state, "Pending" | "Ready" | "Retry" | "Rejected") {
                    out.push(explanation(
                        v,
                        format!(
                            "Admission check {name}: {state}. {}",
                            check["message"].as_str().unwrap_or("")
                        ),
                        "unknown",
                        at,
                    ));
                }
            }
        }
    }
    out
}
pub async fn admission(
    client: &Client,
    namespace: &str,
    pod: &Value,
    captured_at: &str,
) -> Source<Explanation> {
    admission_with_deadline(client, namespace, pod, captured_at, Duration::from_secs(15)).await
}
async fn admission_with_deadline(
    client: &Client,
    namespace: &str,
    pod: &Value,
    captured_at: &str,
    deadline: Duration,
) -> Source<Explanation> {
    if namespace.is_empty()
        || pod.pointer("/metadata/namespace").and_then(Value::as_str) != Some(namespace)
    {
        let mut s = source(captured_at);
        unknown(
            &mut s,
            "Selected pod namespace does not match the requested namespace.",
        );
        return s;
    }
    tokio::time::timeout(deadline, collect(client, namespace, pod, captured_at))
        .await
        .unwrap_or_else(|_| {
            let mut s = source(captured_at);
            s.state = SourceState::Error;
            unknown(&mut s, "Kueue evidence timed out; evidence is incomplete.");
            s
        })
}
async fn collect(client: &Client, ns: &str, pod: &Value, at: &str) -> Source<Explanation> {
    let mut s = source(at);
    let groups = match client.list_api_groups().await {
        Ok(g) => g,
        Err(e) => {
            failure(&mut s, e, true);
            return s;
        }
    };
    let Some(group) = groups.groups.iter().find(|g| g.name == GROUP) else {
        s.state = SourceState::Unsupported;
        unknown(&mut s, "Kueue API is not served.");
        return s;
    };
    let Some(version) = ["v1beta2", "v1beta1"]
        .into_iter()
        .find(|v| group.versions.iter().any(|x| x.version == *v))
    else {
        s.state = SourceState::Unsupported;
        unknown(&mut s, "No supported Kueue API version is served.");
        return s;
    };
    let owners = match owners(client, ns, pod).await {
        Ok(o) => o,
        Err(OwnershipError::Read(error)) => {
            failure(&mut s, error, false);
            return s;
        }
        Err(OwnershipError::Relationship) => {
            unknown(&mut s, "Controller ownership is unresolved or ambiguous.");
            return s;
        }
    };
    if owners.is_empty() {
        return s;
    }
    let a = api(client, ns, GROUP, version, "Workload", "workloads");
    let mut workloads = vec![];
    let mut token = String::new();
    for page in 0..20 {
        let mut lp = ListParams::default().limit(500);
        if !token.is_empty() {
            lp = lp.continue_token(&token)
        }
        let objects = match a.list(&lp).await {
            Ok(o) => o,
            Err(e) => {
                failure(&mut s, e, true);
                return s;
            }
        };
        token = objects.metadata.continue_.unwrap_or_default();
        for o in objects.items {
            if workloads.len() >= 10000 {
                unknown(
                    &mut s,
                    "Workload collection truncated; ownership is unknown.",
                );
                return s;
            }
            workloads.push(serde_json::to_value(o).unwrap_or(Value::Null));
        }
        if token.is_empty() {
            break;
        }
        if page == 19 {
            unknown(
                &mut s,
                "Workload collection truncated; ownership is unknown.",
            );
            return s;
        }
    }
    let Some(w) = matching_workload(&workloads, &owners) else {
        unknown(
            &mut s,
            "No unique UID-verified Kueue Workload relationship was established.",
        );
        return s;
    };
    s.items.extend(conditions(w, at));
    let mut refs = HashSet::new();
    if let Some(q) = w.pointer("/spec/queueName").and_then(Value::as_str) {
        refs.insert((ns.to_owned(), "LocalQueue", "localqueues", q.to_owned()));
    }
    if let Some(q) = w
        .pointer("/status/admission/clusterQueue")
        .and_then(Value::as_str)
    {
        refs.insert((String::new(), "ClusterQueue", "clusterqueues", q.to_owned()));
    }
    for p in w
        .pointer("/status/admission/podSetAssignments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for flavor in p["flavors"]
            .as_object()
            .into_iter()
            .flat_map(|m| m.values())
            .filter_map(Value::as_str)
        {
            refs.insert((
                String::new(),
                "ResourceFlavor",
                "resourceflavors",
                flavor.into(),
            ));
        }
    }
    for c in w
        .pointer("/status/admissionChecks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = c["name"].as_str() {
            refs.insert((
                String::new(),
                "AdmissionCheck",
                "admissionchecks",
                name.into(),
            ));
        }
    }
    let mut pending: Vec<_> = refs.into_iter().collect();
    let mut read = HashSet::new();
    let mut count = 0;
    while let Some((namespace, kind, plural, name)) = pending.pop() {
        if !read.insert((kind, name.clone())) {
            continue;
        }
        if count == 10000 {
            unknown(&mut s, "Referenced Kueue collection truncated.");
            break;
        }
        count += 1;
        match api(client, &namespace, GROUP, version, kind, plural)
            .get(&name)
            .await
        {
            Ok(o) => {
                let v = serde_json::to_value(o).unwrap_or(Value::Null);
                if kind == "LocalQueue" {
                    if let Some(q) = v.pointer("/spec/clusterQueue").and_then(Value::as_str) {
                        pending.push((String::new(), "ClusterQueue", "clusterqueues", q.into()));
                    }
                }
                s.items.extend(conditions(&v, at));
            }
            Err(e) => failure(&mut s, e, false),
        }
    }
    if s.items.is_empty() {
        s.items.push(explanation(w,"UID-verified Workload found, but no current admission condition establishes a blocker.".into(),"unknown",at));
    }
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::gpu_types::ResourceRef;
    use serde_json::json;
    fn owner() -> ResourceRef {
        ResourceRef {
            api_version: "batch/v1".into(),
            kind: "Job".into(),
            namespace: Some("team".into()),
            name: "train".into(),
            uid: "selected".into(),
        }
    }
    fn workload() -> Value {
        json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"Workload","metadata":{"name":"w","namespace":"team","uid":"w","generation":3,"ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":"train","uid":"selected","controller":true}]},"status":{"conditions":[{"type":"QuotaReserved","status":"False","observedGeneration":3,"message":"quota exhausted"}]}})
    }
    #[test]
    fn kueue_matches_only_unique_strict_owner() {
        let w = workload();
        assert!(matching_workload(std::slice::from_ref(&w), &[owner()]).is_some());
        assert!(matching_workload(&[w.clone(), w.clone()], &[owner()]).is_none());
        for field in ["namespace", "uid"] {
            let mut other = w.clone();
            other["metadata"][field] = json!("other");
            if field == "namespace" {
                assert!(matching_workload(&[other], &[owner()]).is_none());
            }
        }
        let mut other = w;
        other["metadata"]["ownerReferences"][0]["controller"] = json!(false);
        assert!(matching_workload(&[other], &[owner()]).is_none());
    }
    #[test]
    fn workload_name_does_not_establish_membership() {
        let workloads = vec![
            json!({"metadata":{"name":"train","namespace":"team","ownerReferences":[{"apiVersion":"batch/v1","kind":"Job","name":"train","uid":"other","controller":true}]}}),
        ];
        let owners = vec![ResourceRef {
            api_version: "batch/v1".into(),
            kind: "Job".into(),
            namespace: Some("team".into()),
            name: "train".into(),
            uid: "selected".into(),
        }];
        assert!(matching_workload(&workloads, &owners).is_none());
    }
}
#[cfg(test)]
mod reader_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    fn client(s: &MockServer) -> Client {
        Client::try_from(kube::Config::new(s.uri().parse().unwrap())).unwrap()
    }
    async fn response(s: &MockServer, p: &str, code: u16, v: Value) {
        Mock::given(path(p))
            .respond_with(ResponseTemplate::new(code).set_body_json(v))
            .mount(s)
            .await;
    }
    fn reference(name: &str, uid: &str) -> Value {
        json!({"apiVersion":"batch/v1","kind":"Job","name":name,"uid":uid,"controller":true})
    }
    fn object(name: &str, uid: &str, next: Option<Value>) -> Value {
        json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":name,"uid":uid,"namespace":"team","ownerReferences":next.into_iter().collect::<Vec<_>>()}})
    }
    fn pod() -> Value {
        json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"pod","uid":"pod","namespace":"team","ownerReferences":[reference("train","job")]}})
    }
    fn workload() -> Value {
        json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"Workload","metadata":{"name":"w","namespace":"team","uid":"w","generation":3,"ownerReferences":[reference("train","job")]},"spec":{"queueName":"local"},"status":{"conditions":[{"type":"QuotaReserved","status":"False","observedGeneration":3,"message":"quota exhausted password=\"private secret\""}],"admissionChecks":[{"name":"check","state":"Pending","message":"await approval"}],"admission":{"clusterQueue":"cluster","podSetAssignments":[{"name":"main","flavors":{"nvidia.com/gpu":"gpu"}}]}}})
    }
    async fn setup(s: &MockServer, versions: &[&str], workloads: Value) {
        response(s,"/apis",200,json!({"apiVersion":"v1","kind":"APIGroupList","groups":[{"name":GROUP,"versions":versions.iter().map(|v|json!({"groupVersion":format!("{GROUP}/{v}"),"version":v})).collect::<Vec<_>>()}]})).await;
        response(
            s,
            "/apis/batch/v1/namespaces/team/jobs/train",
            200,
            object("train", "job", None),
        )
        .await;
        for v in versions {
            response(s,&format!("/apis/{GROUP}/{v}/namespaces/team/workloads"),200,json!({"apiVersion":format!("{GROUP}/{v}"),"kind":"WorkloadList","metadata":{},"items":workloads})).await;
        }
    }
    #[tokio::test]
    async fn kueue_known_versions_reference_only_reads_and_redaction() {
        for version in ["v1beta2", "v1beta1"] {
            let s = MockServer::start().await;
            setup(&s, &[version], json!([workload()])).await;
            for (scope, plural, name, kind) in [
                ("namespaces/team/", "localqueues", "local", "LocalQueue"),
                ("", "clusterqueues", "cluster", "ClusterQueue"),
                ("", "resourceflavors", "gpu", "ResourceFlavor"),
                ("", "admissionchecks", "check", "AdmissionCheck"),
            ] {
                response(&s,&format!("/apis/{GROUP}/{version}/{scope}{plural}/{name}"),200,json!({"apiVersion":format!("{GROUP}/{version}"),"kind":kind,"metadata":{"name":name,"uid":name,"generation":1},"status":{"conditions":[{"type":"Ready","status":"False","observedGeneration":1,"message":"pending"}]}})).await;
            }
            let out = admission(&client(&s), "team", &pod(), "now").await;
            assert!(out.complete, "{:?}", out);
            assert!(out
                .items
                .iter()
                .any(|e| e.message.contains("QuotaReserved=False") && e.confidence == "observed"));
            assert!(out
                .items
                .iter()
                .any(|e| e.message.contains("Admission check check: Pending")));
            assert!(!serde_json::to_string(&out).unwrap().contains("private"));
            let requests = s.received_requests().await.unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.url.path().contains("workloads"))
                    .count(),
                1
            );
            assert!(!requests
                .iter()
                .any(|r| r.url.path().ends_with("resourceflavors")));
        }
    }
    #[tokio::test]
    async fn kueue_prefers_v1beta2_and_discovery_denial_is_safe() {
        let s = MockServer::start().await;
        setup(&s, &["v1beta1", "v1beta2"], json!([])).await;
        let out = admission(&client(&s), "team", &pod(), "now").await;
        assert!(!out.complete);
        assert!(s
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().contains("v1beta2/namespaces/team/workloads")));
        assert!(!s
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().contains("v1beta1/namespaces/team/workloads")));
        let s = MockServer::start().await;
        response(&s,"/apis",403,json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"Forbidden","code":403,"message":"private"})).await;
        let out = admission(&client(&s), "team", &pod(), "now").await;
        assert_eq!(out.state, SourceState::Forbidden);
        assert!(!serde_json::to_string(&out).unwrap().contains("private"));
    }
    #[tokio::test]
    async fn kueue_wrong_uid_ambiguous_and_cross_namespace_are_unknown() {
        for case in 0..4 {
            let s = MockServer::start().await;
            let mut w = workload();
            w["spec"] = json!({});
            w["status"] = json!({});
            if case == 0 {
                w["metadata"]["ownerReferences"][0]["uid"] = json!("other")
            }
            if case == 1 {
                w["metadata"]["namespace"] = json!("other")
            }
            if case == 2 {
                w["metadata"]["ownerReferences"][0]["controller"] = json!(false)
            }
            let ws = if case == 3 {
                json!([w.clone(), w])
            } else {
                json!([w])
            };
            setup(&s, &["v1beta2"], ws).await;
            let out = admission(&client(&s), "team", &pod(), "now").await;
            assert!(!out.complete);
            assert!(out.items.is_empty());
        }
    }
    #[tokio::test]
    async fn kueue_verifies_ancestor_uid_and_detects_cycles_and_four_hop_limit() {
        for (length, cycle, wrong, success) in [
            (4, false, false, true),
            (5, false, false, false),
            (2, true, false, false),
            (1, false, true, false),
        ] {
            let s = MockServer::start().await;
            let mut p = pod();
            p["metadata"]["ownerReferences"] = json!([reference("j0", "u0")]);
            for i in 0..length {
                let next = if i + 1 < length {
                    Some(reference(&format!("j{}", i + 1), &format!("u{}", i + 1)))
                } else if cycle {
                    Some(reference("j0", "u0"))
                } else {
                    None
                };
                response(
                    &s,
                    &format!("/apis/batch/v1/namespaces/team/jobs/j{i}"),
                    200,
                    object(
                        &format!("j{i}"),
                        &if wrong {
                            "replacement".to_owned()
                        } else {
                            format!("u{i}")
                        },
                        next,
                    ),
                )
                .await;
            }
            let result = owners(&client(&s), "team", &p).await;
            assert_eq!(result.is_ok(), success);
            assert!(s.received_requests().await.unwrap().len() <= 4);
        }
    }
    #[tokio::test]
    async fn kueue_ancestor_read_failures_preserve_source_state_and_scheduling() {
        for (code, state) in [
            (403, SourceState::Forbidden),
            (500, SourceState::Error),
            (404, SourceState::Error),
        ] {
            let server = MockServer::start().await;
            setup(&server, &["v1beta2"], json!([])).await;
            Mock::given(path("/apis/batch/v1/namespaces/team/jobs/train"))
                .respond_with(ResponseTemplate::new(code).set_body_json(json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"Failure","code":code,"message":"private error body"})))
                .with_priority(1).mount(&server).await;
            let out = admission(&client(&server), "team", &pod(), "now").await;
            assert_eq!(out.state, state);
            assert!(!out.complete);
            let mut selected = pod();
            selected["status"] = json!({"conditions":[{"type":"PodScheduled","status":"False","message":"Insufficient nvidia.com/gpu"}]});
            response(&server, "/api/v1/namespaces/team/pods/pod", 200, selected).await;
            response(
                &server,
                "/api/v1/namespaces/team/events",
                200,
                json!({"apiVersion":"v1","kind":"List","metadata":{},"items":[]}),
            )
            .await;
            response(
                &server,
                "/api/v1/nodes",
                200,
                json!({"apiVersion":"v1","kind":"List","metadata":{},"items":[]}),
            )
            .await;
            let integrated =
                super::super::gpu_scheduling::snapshot(&client(&server), "team", "pod", "pod")
                    .await
                    .unwrap();
            assert_eq!(integrated.sources["kueue"].state, state);
            assert_eq!(integrated.sources["events"].state, SourceState::Available);
            assert!(integrated
                .explanations
                .iter()
                .any(|e| e.stage == "scheduling" && e.message.contains("Insufficient")));
            assert!(!serde_json::to_string(&integrated)
                .unwrap()
                .contains("private error body"));
        }
    }
    #[tokio::test]
    async fn kueue_continuation_and_collection_cap_are_incomplete_when_truncated() {
        let server = MockServer::start().await;
        let mut w = workload();
        w["spec"] = json!({});
        w["status"] = json!({});
        setup(&server, &["v1beta2"], json!([])).await;
        let endpoint = format!("/apis/{GROUP}/v1beta2/namespaces/team/workloads");
        Mock::given(path(&endpoint)).respond_with(ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"WorkloadList","metadata":{"continue":"next"},"items":[]}))).with_priority(2).mount(&server).await;
        Mock::given(path(&endpoint)).and(wiremock::matchers::query_param("continue","next")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"WorkloadList","metadata":{},"items":[w]}))).with_priority(1).mount(&server).await;
        let out = admission(&client(&server), "team", &pod(), "now").await;
        assert!(out.complete);
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.url.path() == endpoint)
                .count(),
            2
        );
        let server = MockServer::start().await;
        setup(&server, &["v1beta2"], json!([])).await;
        Mock::given(path(&endpoint)).respond_with(ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"WorkloadList","metadata":{"continue":"next"},"items":vec![json!({"metadata":{"name":"unrelated","namespace":"team","uid":"u"}});500]}))).with_priority(1).mount(&server).await;
        let out = admission(&client(&server), "team", &pod(), "now").await;
        assert!(!out.complete);
        assert!(out.message.unwrap().contains("truncated"));
        let requests = server.received_requests().await.unwrap();
        let pages: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path() == endpoint)
            .collect();
        assert_eq!(pages.len(), 20);
        assert!(pages
            .iter()
            .all(|r| r.url.query_pairs().any(|(k, v)| k == "limit" && v == "500")));
    }
    #[tokio::test]
    async fn kueue_deadline_covers_discovery_and_ancestor_reads_together() {
        let server = MockServer::start().await;
        setup(&server, &["v1beta2"], json!([])).await;
        Mock::given(path("/apis")).respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(60)).set_body_json(json!({"apiVersion":"v1","kind":"APIGroupList","groups":[{"name":GROUP,"versions":[{"groupVersion":format!("{GROUP}/v1beta2"),"version":"v1beta2"}]}]}))).with_priority(1).mount(&server).await;
        Mock::given(path("/apis/batch/v1/namespaces/team/jobs/train"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(60))
                    .set_body_json(object("train", "job", None)),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        let out = admission_with_deadline(
            &client(&server),
            "team",
            &pod(),
            "now",
            Duration::from_millis(100),
        )
        .await;
        assert_eq!(out.state, SourceState::Error);
        assert!(!out.complete);
        assert!(out.message.unwrap().contains("timed out"));
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/jobs/train")));
        assert!(!server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/workloads")));
    }
    #[tokio::test]
    async fn kueue_replacement_during_workload_read_invalidates_entire_snapshot() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let server = MockServer::start().await;
        setup(&server, &["v1beta2"], json!([])).await;
        let replaced = Arc::new(AtomicBool::new(false));
        let flag = replaced.clone();
        Mock::given(path(format!("/apis/{GROUP}/v1beta2/namespaces/team/workloads"))).respond_with(move |_:&wiremock::Request| {flag.store(true,Ordering::SeqCst);ResponseTemplate::new(200).set_body_json(json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"WorkloadList","metadata":{},"items":[]}))}).with_priority(1).mount(&server).await;
        let flag = replaced.clone();
        Mock::given(path("/api/v1/namespaces/team/pods/pod"))
            .respond_with(move |_: &wiremock::Request| {
                let mut p = pod();
                if flag.load(Ordering::SeqCst) {
                    p["metadata"]["uid"] = json!("replacement");
                }
                ResponseTemplate::new(200).set_body_json(p)
            })
            .mount(&server)
            .await;
        for endpoint in ["/api/v1/namespaces/team/events", "/api/v1/nodes"] {
            response(
                &server,
                endpoint,
                200,
                json!({"apiVersion":"v1","kind":"List","metadata":{},"items":[]}),
            )
            .await;
        }
        assert!(matches!(
            super::super::gpu_scheduling::snapshot(&client(&server), "team", "pod", "pod").await,
            Err(crate::error::AppError::Conflict(_))
        ));
        assert!(replaced.load(Ordering::SeqCst));
    }
    #[test]
    fn kueue_stale_missing_generation_and_other_object_generation() {
        let mut w = workload();
        w["status"]["admissionChecks"] = json!([]);
        w["status"]["conditions"][0]["observedGeneration"] = json!(2);
        assert_eq!(conditions(&w, "now")[0].confidence, "unknown");
        w["status"]["conditions"][0]["observedGeneration"] = json!(3);
        assert_eq!(conditions(&w, "now")[0].confidence, "observed");
        w["metadata"]["generation"] = Value::Null;
        assert_eq!(conditions(&w, "now")[0].confidence, "unknown");
    }
}
#[cfg(test)]
mod safety_tests {
    use super::*;
    use serde_json::json;
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    #[tokio::test]
    async fn kueue_namespace_mismatch_issues_no_requests() {
        let server = MockServer::start().await;
        let client = Client::try_from(kube::Config::new(server.uri().parse().unwrap())).unwrap();
        let out = admission(
            &client,
            "other",
            &json!({"metadata":{"namespace":"team","uid":"p"}}),
            "now",
        )
        .await;
        assert!(!out.complete);
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn kueue_api_absence_is_unsupported() {
        let server = MockServer::start().await;
        Mock::given(path("/apis"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"apiVersion":"v1","kind":"APIGroupList","groups":[]})),
            )
            .mount(&server)
            .await;
        let client = Client::try_from(kube::Config::new(server.uri().parse().unwrap())).unwrap();
        let out = admission(
            &client,
            "team",
            &json!({"metadata":{"namespace":"team","uid":"p"}}),
            "now",
        )
        .await;
        assert_eq!(out.state, SourceState::Unsupported);
    }
}
