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
    pub available_families: Vec<GpuMetricFamily>,
    pub identity_labels: Vec<String>,
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
    let mut available_families = Vec::new();
    let mut identity_labels = Vec::new();
    if allowed {
        let suffix = format!(
            "/api/v1/query?query={}",
            encode_query_component(&history_query(config, None))
        );
        let bytes = proxy_get(client, config, &suffix).await?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| invalid_history())?;
        if value["status"] != "success" || value["data"]["resultType"] != "vector" {
            return Err(invalid_history());
        }
        let results = value["data"]["result"]
            .as_array()
            .ok_or_else(invalid_history)?;
        if results.len() > 200 {
            return Err(invalid_history());
        }
        for item in results {
            let labels = item["metric"].as_object().ok_or_else(invalid_history)?;
            let name = labels
                .get("__name__")
                .and_then(|v| v.as_str())
                .ok_or_else(invalid_history)?;
            let family = FAMILIES
                .iter()
                .find(|(_, n)| *n == name)
                .map(|(f, _)| *f)
                .ok_or_else(invalid_history)?;
            if !available_families.contains(&family) {
                available_families.push(family);
            }
            for key in [
                "UUID",
                "namespace",
                "pod",
                "pod_uid",
                "container",
                "container_id",
            ] {
                if labels
                    .get(key)
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| !v.is_empty())
                    && !identity_labels.iter().any(|k| k == key)
                {
                    identity_labels.push(key.into());
                }
            }
        }
    }
    Ok(GpuTelemetryCapabilities {
        allowed,
        available_families,
        identity_labels,
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
            Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"success","data":{"resultType":"vector","result":[]}}))).mount(&server).await;
            let result = capabilities(&client, &config).await.unwrap();
            assert_eq!(result.allowed, allowed);
            assert!(!serde_json::to_string(&result).unwrap().contains("SECRET"));
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn gpu_history_windows() {
        for w in [3600, 21600, 86400, 604800] {
            assert!(validate_window(w, 1000000.0, 1000000.0).is_ok());
        }
        for e in [-1.0, f64::NAN, f64::INFINITY, 1000001.0] {
            assert!(validate_window(3600, e, 1000000.0).is_err());
        }
        assert!(validate_window(20, 1000000.0, 1000000.0).is_err());
    }
    #[test]
    fn gpu_history_gaps_and_identity() {
        let c = GpuTelemetryConfig {
            namespace: "m".into(),
            service: "p".into(),
            port: "90".into(),
            cluster_label: None,
            cluster_value: None,
            single_cluster_acknowledged: true,
        };
        let v = serde_json::json!({"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":"DCGM_FI_DEV_FB_USED","UUID":"old-device","secret":"hidden"},"values":[[1,"2"],[2,"NaN"],[3,"+Inf"]]}]}});
        let s = parse_series(&serde_json::to_vec(&v).unwrap(), &c, 0.0, 10.0).unwrap();
        assert_eq!(s[0].points[0].1, Some(2097152.0));
        assert_eq!(s[0].points[1].1, None);
        assert_eq!(s[0].points[2].1, None);
        assert!(!s[0].labels.contains_key("secret"));
        assert_eq!(s[0].labels["lumen_cluster_provenance"], "unverified");
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GpuMetricFamily {
    Utilization,
    FramebufferUsed,
    FramebufferTotal,
    SmActive,
    TensorActive,
    XidErrors,
}
const FAMILIES: [(GpuMetricFamily, &str); 6] = [
    (GpuMetricFamily::Utilization, "DCGM_FI_DEV_GPU_UTIL"),
    (GpuMetricFamily::FramebufferUsed, "DCGM_FI_DEV_FB_USED"),
    (GpuMetricFamily::FramebufferTotal, "DCGM_FI_DEV_FB_TOTAL"),
    (GpuMetricFamily::SmActive, "DCGM_FI_PROF_SM_ACTIVE"),
    (
        GpuMetricFamily::TensorActive,
        "DCGM_FI_PROF_PIPE_TENSOR_ACTIVE",
    ),
    (GpuMetricFamily::XidErrors, "DCGM_FI_DEV_XID_ERRORS"),
];
#[derive(Debug, Clone, Serialize)]
pub struct GpuSeries {
    pub family: GpuMetricFamily,
    pub labels: std::collections::HashMap<String, String>,
    pub points: Vec<(f64, Option<f64>)>,
}
#[derive(Debug, Serialize)]
pub struct GpuHistory {
    pub captured_at: String,
    pub complete: bool,
    pub warnings: Vec<String>,
    pub series: Vec<GpuSeries>,
}
fn invalid_history() -> AppError {
    AppError::K8s("Telemetry history response is invalid or exceeds bounded limits.".into())
}
fn validate_window(window: u64, end: f64, now: f64) -> AppResult<f64> {
    if !matches!(window, 3600 | 21600 | 86400 | 604800)
        || !end.is_finite()
        || end < window as f64
        || end > now
    {
        return Err(AppError::K8s(
            "Choose an allowed history window and a finite past end timestamp.".into(),
        ));
    }
    Ok(end - window as f64)
}
fn history_query(config: &GpuTelemetryConfig, namespace: Option<&str>) -> String {
    let names = FAMILIES
        .iter()
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join("|");
    let mut selectors = vec![format!("__name__=~\"{names}\"")];
    let cluster = exact_cluster_selector(config);
    if !cluster.is_empty() {
        selectors.push(cluster);
    }
    if let Some(ns) = namespace {
        selectors.push(format!("namespace={}", serde_json::to_string(ns).unwrap()));
    }
    format!("{{{}}}", selectors.join(","))
}
fn parse_series(
    bytes: &[u8],
    config: &GpuTelemetryConfig,
    start: f64,
    end: f64,
) -> AppResult<Vec<GpuSeries>> {
    if bytes.len() > RESPONSE_LIMIT {
        return Err(invalid_history());
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid_history())?;
    if value["status"] != "success" || value["data"]["resultType"] != "matrix" {
        return Err(invalid_history());
    }
    let results = value["data"]["result"]
        .as_array()
        .ok_or_else(invalid_history)?;
    if results.len() > 200 {
        return Err(invalid_history());
    }
    results
        .iter()
        .map(|s| {
            let metric = s["metric"].as_object().ok_or_else(invalid_history)?;
            let name = metric
                .get("__name__")
                .and_then(|v| v.as_str())
                .ok_or_else(invalid_history)?;
            let family = FAMILIES
                .iter()
                .find(|(_, n)| *n == name)
                .map(|(f, _)| *f)
                .ok_or_else(invalid_history)?;
            let mut labels = std::collections::HashMap::new();
            for key in [
                "UUID",
                "gpu",
                "device",
                "Hostname",
                "node",
                "namespace",
                "pod",
                "pod_uid",
                "container",
                "container_id",
                "GPU_I_ID",
                "GPU_I_PROFILE",
            ] {
                if let Some(v) = metric.get(key).and_then(|v| v.as_str()) {
                    labels.insert(key.into(), v.into());
                }
            }
            if let (Some(key), Some(expected)) = (&config.cluster_label, &config.cluster_value) {
                if metric
                    .get(key)
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| v != expected)
                {
                    return Err(invalid_history());
                }
            }
            let verified = match (&config.cluster_label, &config.cluster_value) {
                (Some(k), Some(v)) => metric.get(k).and_then(|x| x.as_str()) == Some(v.as_str()),
                _ => false,
            };
            labels.insert(
                "lumen_cluster_provenance".into(),
                if verified { "verified" } else { "unverified" }.into(),
            );
            let values = s["values"].as_array().ok_or_else(invalid_history)?;
            if values.len() > 1000 {
                return Err(invalid_history());
            }
            let mut last = None;
            let mut points = Vec::new();
            for point in values {
                let pair = point.as_array().ok_or_else(invalid_history)?;
                if pair.len() != 2 {
                    return Err(invalid_history());
                }
                let t = pair[0].as_f64().ok_or_else(invalid_history)?;
                if !t.is_finite() || t < start || t > end || last.is_some_and(|l| t <= l) {
                    return Err(invalid_history());
                }
                last = Some(t);
                let raw = pair[1]
                    .as_str()
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite());
                let normalized = raw.and_then(|v| {
                    let n = match family {
                        GpuMetricFamily::FramebufferUsed | GpuMetricFamily::FramebufferTotal => {
                            v * 1048576.0
                        }
                        GpuMetricFamily::SmActive | GpuMetricFamily::TensorActive => v * 100.0,
                        _ => v,
                    };
                    n.is_finite().then_some(n)
                });
                points.push((t, normalized));
            }
            Ok(GpuSeries {
                family,
                labels,
                points,
            })
        })
        .collect()
}
pub async fn history(
    client: &Client,
    config: &GpuTelemetryConfig,
    namespace: Option<&str>,
    window: u64,
    end: f64,
) -> AppResult<GpuHistory> {
    let now = chrono::Utc::now();
    let start = validate_window(window, end, now.timestamp_millis() as f64 / 1000.0)?;
    // Empty namespace matches missing labels too, retaining unallocated/disappeared devices.
    let scopes = match namespace {
        Some(ns) if !ns.is_empty() => vec![Some(ns), Some("")],
        _ => vec![None],
    };
    let series = tokio::time::timeout(REQUEST_TIMEOUT, async {
        let mut series = Vec::new();
        let mut bytes_total = 0usize;
        for scope in scopes {
            let query = history_query(config, scope);
            let suffix = format!(
                "/api/v1/query_range?query={}&start={start}&end={end}&step={}",
                encode_query_component(&query),
                window.div_ceil(999)
            );
            let bytes = proxy_get(client, config, &suffix).await?;
            bytes_total += bytes.len();
            if bytes_total > RESPONSE_LIMIT {
                return Err(invalid_history());
            }
            let parsed = parse_series(&bytes, config, start, end)?;
            // Enforce requested namespace again even if a backend ignored its selector.
            if let Some(ns) = scope {
                if parsed
                    .iter()
                    .any(|s| s.labels.get("namespace").map(String::as_str).unwrap_or("") != ns)
                {
                    return Err(invalid_history());
                }
            }
            series.extend(parsed);
            if series.len() > 200 {
                return Err(invalid_history());
            }
        }
        Ok(series)
    })
    .await
    .map_err(|_| AppError::K8s("Telemetry request timed out after 15 seconds.".into()))??;
    let mut warnings=vec!["Framebuffer values are bytes; utilization/profiling values are percentages. XID values are error observations, not error counts or root causes.".into()];
    if series
        .iter()
        .any(|s| s.labels["lumen_cluster_provenance"] != "verified")
    {
        warnings.push(
            "Selected Service is declared provenance; cluster identity is unverified.".into(),
        );
    }
    Ok(GpuHistory {
        captured_at: now.to_rfc3339(),
        complete: true,
        warnings,
        series,
    })
}

#[cfg(test)]
mod history_bounds_tests {
    use super::*;
    fn config() -> GpuTelemetryConfig {
        GpuTelemetryConfig {
            namespace: "m".into(),
            service: "p".into(),
            port: "90".into(),
            cluster_label: Some("cluster".into()),
            cluster_value: Some("a\"}\\\n雪".into()),
            single_cluster_acknowledged: false,
        }
    }
    fn response(name: &str, points: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":name},"values":points}]}})).unwrap()
    }
    #[test]
    fn gpu_history_fixed_families_and_selectors() {
        let c = config();
        let q = history_query(&c, Some("team\"}\\\n"));
        assert!(q.contains(&exact_cluster_selector(&c)));
        assert!(q.contains("namespace=\"team\\\"}\\\\\\n\""));
        for (_, name) in FAMILIES {
            assert!(q.contains(name));
            assert!(parse_series(
                &response(name, serde_json::json!([[1, "0.5"]])),
                &c,
                0.0,
                2.0
            )
            .is_ok());
        }
        assert!(parse_series(&response("up", serde_json::json!([])), &c, 0.0, 2.0).is_err());
    }
    #[test]
    fn gpu_history_order_and_point_bounds() {
        let c = config();
        for points in [
            serde_json::json!([[1, "1"], [1, "2"]]),
            serde_json::json!([[2, "1"], [1, "2"]]),
            serde_json::json!([[3, "1"]]),
        ] {
            assert!(parse_series(&response(FAMILIES[0].1, points), &c, 0.0, 2.0).is_err());
        }
        let points: Vec<_> = (0..1001).map(|t| serde_json::json!([t, "1"])).collect();
        assert!(parse_series(
            &response(FAMILIES[0].1, serde_json::json!(points)),
            &c,
            0.0,
            2000.0
        )
        .is_err());
    }
    #[test]
    fn gpu_history_series_bounds_and_units() {
        let c = config();
        let item = serde_json::json!({"metric":{"__name__":FAMILIES[0].1},"values":[]});
        let bytes=serde_json::to_vec(&serde_json::json!({"status":"success","data":{"resultType":"matrix","result":vec![item;201]}})).unwrap();
        assert!(parse_series(&bytes, &c, 0.0, 2.0).is_err());
        let s = parse_series(
            &response(FAMILIES[3].1, serde_json::json!([[1, "0.5"]])),
            &c,
            0.0,
            2.0,
        )
        .unwrap();
        assert_eq!(s[0].points[0].1, Some(50.0));
        for w in [3600_u64, 21600, 86400, 604800] {
            assert!(w / w.div_ceil(999) + 1 <= 1000);
        }
    }
}

#[cfg(test)]
mod history_transport_tests {
    use super::*;
    #[tokio::test]
    async fn gpu_history_range_device_retention_and_namespace() {
        use wiremock::{
            matchers::{method, path, query_param},
            Mock, ResponseTemplate,
        };
        let server = wiremock::MockServer::start().await;
        let client = Client::try_from(kube::Config::new(server.uri().parse().unwrap())).unwrap();
        let c = GpuTelemetryConfig {
            namespace: "m".into(),
            service: "p".into(),
            port: "90".into(),
            cluster_label: None,
            cluster_value: None,
            single_cluster_acknowledged: true,
        };
        let end = chrono::Utc::now().timestamp() as f64 - 1.0;
        for (scope, id) in [("team", "workload"), ("", "deleted-device")] {
            Mock::given(method("GET")).and(path("/api/v1/namespaces/m/services/p:90/proxy/api/v1/query_range")).and(query_param("query",history_query(&c,Some(scope)))).and(query_param("step","4")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":"DCGM_FI_DEV_GPU_UTIL","namespace":scope,"UUID":id},"values":[[end,"NaN"]]}]}}))).expect(1).mount(&server).await;
        }
        let h = history(&client, &c, Some("team"), 3600, end).await.unwrap();
        assert_eq!(h.series.len(), 2);
        assert!(h
            .series
            .iter()
            .any(|s| s.labels["UUID"] == "deleted-device"));
        assert!(h.series.iter().all(|s| s.points[0].1.is_none()));
        assert!(!h.warnings.is_empty());
    }
}
#[cfg(test)]
mod provenance_tests {
    use super::*;
    #[test]
    fn gpu_history_native_cluster_identity() {
        let c = GpuTelemetryConfig {
            namespace: "m".into(),
            service: "p".into(),
            port: "90".into(),
            cluster_label: Some("cluster".into()),
            cluster_value: Some("selected".into()),
            single_cluster_acknowledged: false,
        };
        for cluster in ["selected", "other"] {
            let bytes=serde_json::to_vec(&serde_json::json!({"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":"DCGM_FI_DEV_GPU_UTIL","cluster":cluster,"lumen_cluster_provenance":"verified"},"values":[]}]}})).unwrap();
            let parsed = parse_series(&bytes, &c, 0.0, 1.0);
            if cluster == "selected" {
                assert_eq!(
                    parsed.unwrap()[0].labels["lumen_cluster_provenance"],
                    "verified"
                );
            } else {
                assert!(parsed.is_err());
            }
        }
    }
}
