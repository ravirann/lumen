//! Bounded Kubernetes service-proxy transport; no arbitrary source URLs.
use crate::{
    error::{AppError, AppResult},
    gpu_settings::{validate_config, GpuTelemetryConfig},
};
use http_body::Body as HttpBody;
use kube::{client::Body, Client};
use serde::Serialize;
use std::{pin::Pin, time::Duration};
pub const RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub fn encode_query_component(input: &str) -> String {
    let mut result = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            result.push(b as char)
        } else {
            use std::fmt::Write;
            write!(&mut result, "%{b:02X}").expect("string write");
        }
    }
    result
}
pub fn exact_cluster_selector(config: &GpuTelemetryConfig) -> String {
    match (&config.cluster_label, &config.cluster_value) {
        (Some(label), Some(value)) => format!(
            "{label}={}",
            serde_json::to_string(value).expect("string serialization")
        ),
        _ => String::new(),
    }
}
fn append_bounded(output: &mut Vec<u8>, next: &[u8]) -> AppResult<()> {
    if output
        .len()
        .checked_add(next.len())
        .is_none_or(|n| n > RESPONSE_LIMIT)
    {
        return Err(AppError::K8s(
            "Telemetry response exceeded 4 MiB; narrow the query.".into(),
        ));
    }
    output.extend_from_slice(next);
    Ok(())
}
fn safe_failure(status: u16) -> AppError {
    match status {
 403 => AppError::PermissionDenied("Telemetry requires get permission on services/proxy in the configured namespace.".into()),
 404 => AppError::NotFound("Telemetry Service or proxy endpoint was not found. Check the configured namespace, Service, and port.".into()),
 _ => AppError::K8s("Telemetry source request failed. Check the Service and Kubernetes connection.".into()),
}
}
fn proxy_path(config: &GpuTelemetryConfig, suffix: &str) -> AppResult<String> {
    validate_config(config)?;
    let endpoint = suffix.split('?').next().unwrap_or("");
    if !matches!(endpoint, "/api/v1/query" | "/api/v1/query_range")
        || suffix.contains('#')
        || suffix.chars().any(char::is_control)
    {
        return Err(AppError::K8s("Unsupported telemetry API request.".into()));
    }
    Ok(format!(
        "/api/v1/namespaces/{}/services/{}:{}/proxy{}",
        config.namespace, config.service, config.port, suffix
    ))
}
pub async fn proxy_get(
    client: &Client,
    config: &GpuTelemetryConfig,
    suffix: &str,
) -> AppResult<Vec<u8>> {
    proxy_get_with_timeout(client, config, suffix, REQUEST_TIMEOUT).await
}
async fn proxy_get_with_timeout(
    client: &Client,
    config: &GpuTelemetryConfig,
    suffix: &str,
    timeout: Duration,
) -> AppResult<Vec<u8>> {
    let path = proxy_path(config, suffix)?;
    tokio::time::timeout(timeout, async {
        let request = http::Request::get(path)
            .body(Body::empty())
            .map_err(|_| safe_failure(0))?;
        let response = client.send(request).await.map_err(|_| safe_failure(0))?;
        if !response.status().is_success() {
            return Err(safe_failure(response.status().as_u16()));
        }
        // kube's gzip middleware streams decoded frames. If decompression was disabled
        // or the source returned an unsupported encoding, reject rather than parse bytes.
        if response
            .headers()
            .get(http::header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        {
            return Err(AppError::K8s(
                "Telemetry response encoding is unsupported.".into(),
            ));
        }
        let mut body = response.into_body();
        let mut output = Vec::new();
        while let Some(frame) =
            futures::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
        {
            let frame = frame.map_err(|_| safe_failure(0))?;
            if let Some(bytes) = frame.data_ref() {
                append_bounded(&mut output, bytes)?;
            }
        }
        Ok(output)
    })
    .await
    .map_err(|_| AppError::K8s("Telemetry request timed out after 15 seconds.".into()))?
}
#[derive(Debug, Clone, Serialize)]
pub struct GpuTelemetryCapabilities {
    pub allowed: bool,
    pub message: Option<String>,
}
pub async fn capabilities(
    client: &Client,
    config: &GpuTelemetryConfig,
) -> AppResult<GpuTelemetryCapabilities> {
    use k8s_openapi::api::authorization::v1::{
        ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    };
    validate_config(config)?;
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(ResourceAttributes {
                group: Some(String::new()),
                version: Some("v1".into()),
                resource: Some("services".into()),
                subresource: Some("proxy".into()),
                verb: Some("get".into()),
                namespace: Some(config.namespace.clone()),
                name: Some(format!("{}:{}", config.service, config.port)),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let api: kube::Api<SelfSubjectAccessReview> = kube::Api::all(client.clone());
    let response=tokio::time::timeout(REQUEST_TIMEOUT,api.create(&kube::api::PostParams::default(),&review)).await.map_err(|_|AppError::K8s("Telemetry permission check timed out.".into()))?.map_err(|_|AppError::K8s("Telemetry permission check failed. Kubernetes will enforce services/proxy access on each request.".into()))?;
    let allowed = response.status.is_some_and(|s| s.allowed);
    Ok(GpuTelemetryCapabilities {
        allowed,
        message: (!allowed).then(|| {
            "Telemetry requires get permission on services/proxy in the configured namespace."
                .into()
        }),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_telemetry_encoding_and_bounds() {
        assert_eq!(encode_query_component("%\"雪+&"), "%25%22%E9%9B%AA%2B%26");
        let mut out = vec![0; 4 * 1024 * 1024];
        assert!(append_bounded(&mut out, &[1]).is_err());
        assert_eq!(out.len(), 4 * 1024 * 1024);
        let c = GpuTelemetryConfig {
            namespace: "m".into(),
            service: "p".into(),
            port: "9090".into(),
            cluster_label: Some("cluster".into()),
            cluster_value: Some("a\"}\\\n雪".into()),
            single_cluster_acknowledged: false,
        };
        assert_eq!(exact_cluster_selector(&c), "cluster=\"a\\\"}\\\\\\n雪\"");
    }
    async fn fixture() -> (wiremock::MockServer, Client, GpuTelemetryConfig) {
        let server = wiremock::MockServer::start().await;
        let client = Client::try_from(kube::Config::new(server.uri().parse().unwrap())).unwrap();
        let config = GpuTelemetryConfig {
            namespace: "monitoring".into(),
            service: "prometheus".into(),
            port: "9090".into(),
            cluster_label: None,
            cluster_value: None,
            single_cluster_acknowledged: true,
        };
        (server, client, config)
    }
    #[tokio::test]
    async fn gpu_telemetry_safe_status_and_success() {
        use wiremock::{
            matchers::{method, path},
            Mock, ResponseTemplate,
        };
        for (status, expected) in [
            (403, "requires get permission on services/proxy"),
            (404, "Service or proxy endpoint was not found"),
            (500, "Telemetry source request failed"),
        ] {
            let (server, client, config) = fixture().await;
            Mock::given(method("GET"))
                .and(path(
                    "/api/v1/namespaces/monitoring/services/prometheus:9090/proxy/api/v1/query",
                ))
                .respond_with(ResponseTemplate::new(status).set_body_string("SECRET_PASSWORD"))
                .mount(&server)
                .await;
            let error = proxy_get(&client, &config, "/api/v1/query?query=up")
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("SECRET"));
        }
        let (server, client, config) = fixture().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        assert_eq!(
            proxy_get(&client, &config, "/api/v1/query?query=up")
                .await
                .unwrap(),
            b"ok"
        );
        for invalid in [
            "/metrics",
            "/api/v1/query/../../secrets",
            "/api/v1/query?query=up#x",
        ] {
            assert!(proxy_get(&client, &config, invalid).await.is_err());
        }
    }
    #[tokio::test]
    async fn gpu_telemetry_decoded_gzip_bound() {
        use std::io::Write;
        use wiremock::{matchers::method, Mock, ResponseTemplate};
        let (server, client, config) = fixture().await;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&vec![b'x'; 4 * 1024 * 1024 + 1]).unwrap();
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "gzip")
                    .set_body_bytes(gzip.finish().unwrap()),
            )
            .mount(&server)
            .await;
        assert!(proxy_get(&client, &config, "/api/v1/query?query=up")
            .await
            .unwrap_err()
            .to_string()
            .contains("4 MiB"));
    }

    #[tokio::test]
    async fn gpu_telemetry_deadline_and_rbac_preflight() {
        use wiremock::{
            matchers::{body_partial_json, method, path},
            Mock, ResponseTemplate,
        };
        let (server, client, config) = fixture().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_string("ok"),
            )
            .mount(&server)
            .await;
        assert!(proxy_get_with_timeout(
            &client,
            &config,
            "/api/v1/query?query=up",
            Duration::from_millis(5)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out"));
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(15));
        for allowed in [false, true] {
            server.reset().await;
            Mock::given(method("POST")).and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews")).and(body_partial_json(serde_json::json!({"spec":{"resourceAttributes":{"resource":"services","subresource":"proxy","verb":"get","namespace":"monitoring","name":"prometheus:9090","group":""}}}))).respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","metadata":{},"spec":{},"status":{"allowed":allowed,"reason":"SECRET_REASON","evaluationError":"SECRET_ERROR"}}))).expect(1).mount(&server).await;
            let result = capabilities(&client, &config).await.unwrap();
            assert_eq!(result.allowed, allowed);
            assert!(!serde_json::to_string(&result).unwrap().contains("SECRET"));
        }
    }
}
