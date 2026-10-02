//! Synthetic NVIDIA extended-resource requests on an isolated disposable fixture.
//! No NVIDIA hardware, drivers, or exporter are installed or implied.
use kube::{
    api::{DynamicObject, Patch, PatchParams},
    config::{KubeConfigOptions, Kubeconfig},
    core::{ApiResource, GroupVersionKind},
    Api, Client, Config,
};
use lumen_lib::k8s::{gpu_inventory::snapshot, gpu_types::SourceState};
use serde_json::{json, Value};

async fn fixture(client: &Client, plural: &str, value: Value) {
    let version = value["apiVersion"].as_str().unwrap();
    let (group, version) = version.split_once('/').unwrap_or(("", version));
    let ar = ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk(group, version, value["kind"].as_str().unwrap()),
        plural,
    );
    let api: Api<DynamicObject> = Api::namespaced_with(
        client.clone(),
        value["metadata"]["namespace"].as_str().unwrap(),
        &ar,
    );
    api.patch(
        value["metadata"]["name"].as_str().unwrap(),
        &PatchParams::apply("lumen-gpu-fixture"),
        &Patch::Apply(&value),
    )
    .await
    .unwrap();
}
#[tokio::test]
#[ignore = "requires disposable scripts/test-kind.sh fixture"]
async fn gpu_inventory_namespace_rbac_terminal_requests_and_redaction() {
    let path = std::env::var("LUMEN_E2E_KUBECONFIG").expect("use scripts/test-kind.sh");
    let kubeconfig = Kubeconfig::read_from(path).unwrap();
    assert!(
        kubeconfig
            .current_context
            .as_deref()
            .unwrap()
            .starts_with("kind-lumen-e2e-"),
        "refusing non-fixture context"
    );
    let config = Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
        .await
        .unwrap();
    assert!(
        matches!(config.cluster_url.host(), Some("127.0.0.1" | "localhost")),
        "refusing non-local server"
    );
    let admin = Client::try_from(config.clone()).unwrap();
    for (namespace, name) in [
        ("lumen-e2e-a", "gpu-pending"),
        ("lumen-e2e-a", "gpu-terminal"),
        ("lumen-e2e-b", "gpu-other-namespace"),
    ] {
        fixture(&admin,"pods",json!({"apiVersion":"v1","kind":"Pod","metadata":{"namespace":namespace,"name":name,"annotations":{"fixture-secret":"must-not-cross-ipc"}},
            "spec":{"schedulerName":"lumen-fixture-disabled","restartPolicy":"Never","containers":[{"name":"worker","image":"registry.k8s.io/pause:3.10","env":[{"name":"SECRET","value":"must-not-cross-ipc"}],"resources":{"limits":{"nvidia.com/gpu":"1"}}}]}})).await;
    }
    let pods: Api<k8s_openapi::api::core::v1::Pod> = Api::namespaced(admin.clone(), "lumen-e2e-a");
    pods.patch_status(
        "gpu-terminal",
        &PatchParams::default(),
        &Patch::Merge(json!({"status":{"phase":"Succeeded"}})),
    )
    .await
    .unwrap();
    fixture(&admin,"roles",json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":"Role","metadata":{"name":"gpu-inventory-reader","namespace":"lumen-e2e-a"},"rules":[{"apiGroups":[""],"resources":["pods"],"verbs":["list"]}]})).await;
    fixture(&admin,"rolebindings",json!({"apiVersion":"rbac.authorization.k8s.io/v1","kind":"RoleBinding","metadata":{"name":"gpu-inventory-reader","namespace":"lumen-e2e-a"},"roleRef":{"apiGroup":"rbac.authorization.k8s.io","kind":"Role","name":"gpu-inventory-reader"},"subjects":[{"kind":"ServiceAccount","name":"viewer","namespace":"lumen-e2e-a"}]})).await;
    let full = snapshot(&admin, "lumen-e2e-a").await;
    assert!(full.pods.complete && full.nodes.complete);
    assert!(full
        .pods
        .items
        .iter()
        .all(|p| p.resource.namespace.as_deref() == Some("lumen-e2e-a")));
    let pending = full
        .pods
        .items
        .iter()
        .find(|p| p.resource.name == "gpu-pending")
        .unwrap();
    assert_eq!(pending.requests["nvidia.com/gpu"], Some(1));
    assert_eq!(
        full.pods
            .items
            .iter()
            .find(|p| p.resource.name == "gpu-terminal")
            .unwrap()
            .phase,
        "Succeeded"
    );
    assert!(!serde_json::to_string(&full)
        .unwrap()
        .contains("must-not-cross-ipc"));
    let mut restricted_config = config;
    restricted_config.auth_info.impersonate =
        Some("system:serviceaccount:lumen-e2e-a:viewer".into());
    let restricted = Client::try_from(restricted_config).unwrap();
    let visible = snapshot(&restricted, "lumen-e2e-a").await;
    assert_eq!(visible.pods.state, SourceState::Available);
    assert!(visible.pods.complete);
    assert_eq!(visible.nodes.state, SourceState::Forbidden);
    assert!(!visible.nodes.complete);
    let denied = snapshot(&restricted, "lumen-e2e-b").await;
    assert_eq!(denied.pods.state, SourceState::Forbidden);
    assert!(denied.pods.items.is_empty());
}
