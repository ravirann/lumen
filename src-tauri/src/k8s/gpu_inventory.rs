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
