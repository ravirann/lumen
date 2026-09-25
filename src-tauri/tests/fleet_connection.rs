//! An isolated kubeconfig and local HTTP fixture; never contacts a real cluster.
use lumen_lib::k8s::{client::K8sState, fleet};
use std::time::Duration;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn slow_inventory_does_not_make_a_connected_cluster_unreachable() {
    let server = MockServer::start().await;
    let config = std::env::temp_dir().join(format!("lumen-fleet-{}.yaml", std::process::id()));
    std::fs::write(&config, format!("contexts: [{{name: fixture, context: {{cluster: c}}}}]\nclusters: [{{name: c, cluster: {{server: {}}}}}]\n", server.uri())).unwrap();
    // This integration-test process has only this test; no concurrent environment users.
    std::env::set_var("KUBECONFIG", &config);
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"metadata":{},"items":[]})),
        )
        .with_priority(10)
        .mount(&server)
        .await;
    Mock::given(path("/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"major":"1","minor":"33","gitVersion":"v1.33.8","gitCommit":"test","gitTreeState":"clean","buildDate":"2026-01-01T00:00:00Z","goVersion":"go1.24","compiler":"gc","platform":"linux/amd64"})))
        .mount(&server).await;
    Mock::given(path("/api/v1/namespaces"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"metadata":{},"items":[{"metadata":{"name":"default"}}]}),
        ))
        .mount(&server)
        .await;
    Mock::given(path("/api/v1/pods"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(40))
                .set_body_json(serde_json::json!({"metadata":{},"items":[]})),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let state = K8sState::default();
    let card = tokio::time::timeout(
        Duration::from_secs(35),
        fleet::probe_context(&state, "fixture"),
    )
    .await
    .expect("inventory must remain bounded")
    .unwrap();
    assert!(
        card.reachable,
        "a successful version request proves connectivity: {:?}",
        card.error
    );
    assert_eq!(card.server_version.as_deref(), Some("v1.33"));
    assert!(
        card.error.as_deref().unwrap_or_default().contains("pods"),
        "missing inventory must be identified"
    );
    assert_eq!(
        card.namespace_count, 1,
        "successful inventory reads are retained"
    );
    let recovered = fleet::probe_context(&state, "fixture").await.unwrap();
    assert!(recovered.reachable);
    assert!(
        recovered.error.is_none(),
        "retry clears the inventory warning: {:?}",
        recovered.error
    );

    server.reset().await;
    Mock::given(path("/version"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({"kind":"Status","apiVersion":"v1","status":"Failure","reason":"Unauthorized","message":"Unauthorized","code":401})))
        .mount(&server).await;
    let denied = fleet::probe_context(&state, "fixture").await.unwrap();
    assert!(!denied.reachable);
    assert!(denied.error.as_deref().unwrap().contains("Unauthorized"));
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "failed authentication must not start inventory requests"
    );

    server.reset().await;
    Mock::given(path("/version"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(40)))
        .mount(&server)
        .await;
    let timed_out = tokio::time::timeout(
        Duration::from_secs(12),
        fleet::probe_context(&state, "fixture"),
    )
    .await
    .expect("connection deadline must remain bounded")
    .unwrap();
    assert!(!timed_out.reachable);
    assert!(timed_out
        .error
        .as_deref()
        .unwrap()
        .contains("connection timed out"));
    std::fs::remove_file(config).unwrap();
}
