//! Structural synthetic Kueue schemas on the isolated kind fixture; no controller/GPU.
use kube::{
    api::{DynamicObject, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig},
    core::{ApiResource, GroupVersionKind},
    Api, Client, Config,
};
use lumen_lib::k8s::{gpu_scheduling, gpu_types::SourceState, kueue};
use serde_json::{json, Value};
use std::time::Duration;
async fn config() -> Config {
    let file = std::env::var("LUMEN_E2E_KUBECONFIG").expect("use scripts/test-kind.sh");
    let kc = Kubeconfig::read_from(file).unwrap();
    assert!(kc
        .current_context
        .as_deref()
        .unwrap()
        .starts_with("kind-lumen-e2e-"));
    let c = Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default())
        .await
        .unwrap();
    assert!(matches!(
        c.cluster_url.host(),
        Some("127.0.0.1" | "localhost")
    ));
    c
}
fn api(c: &Client, ns: &str, g: &str, v: &str, k: &str, p: &str) -> Api<DynamicObject> {
    let ar = ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk(g, v, k), p);
    if ns.is_empty() {
        Api::all_with(c.clone(), &ar)
    } else {
        Api::namespaced_with(c.clone(), ns, &ar)
    }
}
async fn apply(a: &Api<DynamicObject>, v: Value) -> Value {
    serde_json::to_value(
        a.patch(
            v["metadata"]["name"].as_str().unwrap(),
            &PatchParams::apply("lumen-kueue-fixture"),
            &Patch::Apply(&v),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}
#[tokio::test]
#[ignore = "requires disposable fixture created by scripts/test-kind.sh"]
async fn kueue_real_schema_uid_namespace_and_scoped_scheduling() {
    let cfg = config().await;
    let admin = Client::try_from(cfg.clone()).unwrap();
    let conditions = json!({"type":"array","items":{"type":"object","required":["type","status","reason","message","lastTransitionTime"],"properties":{"type":{"type":"string"},"status":{"type":"string","enum":["True","False","Unknown"]},"observedGeneration":{"type":"integer","format":"int64"},"reason":{"type":"string"},"message":{"type":"string"},"lastTransitionTime":{"type":"string","format":"date-time"}}}});
    let crd = json!({"apiVersion":"apiextensions.k8s.io/v1","kind":"CustomResourceDefinition","metadata":{"name":"workloads.kueue.x-k8s.io"},"spec":{"group":"kueue.x-k8s.io","scope":"Namespaced","names":{"plural":"workloads","singular":"workload","kind":"Workload"},"versions":[{"name":"v1beta2","served":true,"storage":true,"schema":{"openAPIV3Schema":{"type":"object","properties":{"spec":{"type":"object","properties":{"queueName":{"type":"string"},"podSets":{"type":"array","items":{"type":"object","required":["name","count","template"],"properties":{"name":{"type":"string"},"count":{"type":"integer","minimum":1},"template":{"type":"object","x-kubernetes-preserve-unknown-fields":true}}}}}},"status":{"type":"object","properties":{"conditions":conditions}}}}}}]}});
    apply(
        &api(
            &admin,
            "",
            "apiextensions.k8s.io",
            "v1",
            "CustomResourceDefinition",
            "customresourcedefinitions",
        ),
        crd,
    )
    .await;
    for _ in 0..30 {
        if admin
            .list_api_group_resources("kueue.x-k8s.io/v1beta2")
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let jobs = api(&admin, "lumen-e2e-a", "batch", "v1", "Job", "jobs");
    // This structural fixture owns its pod manually. Prevent the built-in Job
    // controller from releasing that owner reference for a non-matching selector.
    let job=apply(&jobs,json!({"apiVersion":"batch/v1","kind":"Job","metadata":{"name":"kueue-train","namespace":"lumen-e2e-a"},"spec":{"managedBy":"fixture.example.com/kueue-test","suspend":true,"template":{"spec":{"restartPolicy":"Never","containers":[{"name":"c","image":"registry.k8s.io/pause:3.10"}]}}}})).await;
    let owner = json!({"apiVersion":"batch/v1","kind":"Job","name":"kueue-train","uid":job["metadata"]["uid"],"controller":true});
    let wa = api(
        &admin,
        "lumen-e2e-a",
        "kueue.x-k8s.io",
        "v1beta2",
        "Workload",
        "workloads",
    );
    let w = json!({"apiVersion":"kueue.x-k8s.io/v1beta2","kind":"Workload","metadata":{"name":"kueue-w","namespace":"lumen-e2e-a","ownerReferences":[owner.clone()]},"spec":{"podSets":[{"name":"main","count":1,"template":{"spec":{"restartPolicy":"Never","containers":[{"name":"c","image":"registry.k8s.io/pause:3.10"}]}}}]},"status":{"conditions":[{"type":"QuotaReserved","status":"False","observedGeneration":1,"reason":"Pending","message":"fixture quota exhausted","lastTransitionTime":"2026-10-02T00:00:00Z"}]}});
    apply(&wa, w.clone()).await;
    let mut other = w;
    other["metadata"]["namespace"] = json!("lumen-e2e-b");
    other["status"]["conditions"][0]["message"] = json!("cross namespace must not leak");
    apply(
        &api(
            &admin,
            "lumen-e2e-b",
            "kueue.x-k8s.io",
            "v1beta2",
            "Workload",
            "workloads",
        ),
        other,
    )
    .await;
    apply(&api(&admin,"lumen-e2e-a","","v1","PersistentVolumeClaim","persistentvolumeclaims"),json!({"apiVersion":"v1","kind":"PersistentVolumeClaim","metadata":{"name":"kueue-data","namespace":"lumen-e2e-a"},"spec":{"storageClassName":"no-fixture-provisioner","accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"1Gi"}}}})).await;
    let p=apply(&api(&admin,"lumen-e2e-a","","v1","Pod","pods"),json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":"kueue-pod","namespace":"lumen-e2e-a","ownerReferences":[owner]},"spec":{"schedulingGates":[{"name":"fixture.example.com/hold"}],"containers":[{"name":"c","image":"registry.k8s.io/pause:3.10","resources":{"requests":{"nvidia.com/gpu":"1"},"limits":{"nvidia.com/gpu":"1"}}}],"volumes":[{"name":"data","persistentVolumeClaim":{"claimName":"kueue-data"}}]}})).await;
    let role = api(
        &admin,
        "lumen-e2e-a",
        "rbac.authorization.k8s.io",
        "v1",
        "Role",
        "roles",
    );
    apply(&role,json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":"Role","metadata":{"name":"kueue-viewer","namespace":"lumen-e2e-a"},"rules":[{"apiGroups":[""],"resources":["pods","events","persistentvolumeclaims"],"verbs":["get","list"]},{"apiGroups":["batch"],"resources":["jobs"],"verbs":["get"]},{"apiGroups":["kueue.x-k8s.io"],"resources":["workloads"],"verbs":["get","list"]}]})).await;
    apply(&api(&admin,"lumen-e2e-a","rbac.authorization.k8s.io","v1","RoleBinding","rolebindings"),json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":"RoleBinding","metadata":{"name":"kueue-viewer","namespace":"lumen-e2e-a"},"roleRef":{"apiGroup":"rbac.authorization.k8s.io","kind":"Role","name":"kueue-viewer"},"subjects":[{"kind":"ServiceAccount","name":"viewer","namespace":"lumen-e2e-a"}]})).await;
    let mut viewer = cfg;
    viewer.auth_info.impersonate = Some("system:serviceaccount:lumen-e2e-a:viewer".into());
    let viewer = Client::try_from(viewer).unwrap();
    let out = gpu_scheduling::snapshot(
        &viewer,
        "lumen-e2e-a",
        "kueue-pod",
        p["metadata"]["uid"].as_str().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(out.sources["nodes"].state, SourceState::Forbidden);
    assert_eq!(out.sources["events"].state, SourceState::Available);
    assert_eq!(out.sources["pvcs"].state, SourceState::Available);
    assert_eq!(out.sources["kueue"].state, SourceState::Available);
    assert!(
        out.explanations
            .iter()
            .any(|e| e.message.contains("fixture quota exhausted")),
        "Kueue fixture evidence: {:?}; explanations: {:?}",
        out.sources["kueue"],
        out.explanations
    );
    assert!(!serde_json::to_string(&out)
        .unwrap()
        .contains("must not leak"));
    let denied = kueue::admission(&viewer, "lumen-e2e-b", &p, "now").await;
    assert!(!denied.complete);
}
