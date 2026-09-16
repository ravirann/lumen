//! CRD browser backend.
//!
//! Discovery path:
//! 1. List all CRDs via apiextensions.k8s.io/v1.
//! 2. For a specific CRD, list its custom-resource instances through kube's
//!    dynamic API (`DynamicObject`). No per-CRD code required.
//!
//! Dynamic writes resolve identity from the installed CRD and use optimistic concurrency.

use crate::error::{AppError, AppResult};
use crate::k8s::time;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
    CustomResourceDefinition, CustomResourceDefinitionVersion,
};
use kube::api::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{api::ListParams, Api, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct CrdSummary {
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,
    pub short_names: Vec<String>,
    /// "Namespaced" or "Cluster".
    pub scope: String,
    /// All versions known to the cluster.
    pub versions: Vec<String>,
    /// Preferred served+storage version (falls back to first served).
    pub preferred_version: String,
    pub age_seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrInstance {
    pub name: String,
    pub namespace: Option<String>,
    pub age_seconds: i64,
    /// Small subset of `status` serialized as a short string (when present).
    pub status_hint: Option<String>,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub generation: Option<i64>,
    pub conditions: Vec<CrCondition>,
    pub printer_cells: Vec<PrinterCell>,
}

fn preferred_version(versions: &[CustomResourceDefinitionVersion]) -> String {
    if let Some(v) = versions.iter().find(|v| v.storage && v.served) {
        return v.name.clone();
    }
    versions
        .iter()
        .find(|v| v.served)
        .map(|v| v.name.clone())
        .or_else(|| versions.first().map(|v| v.name.clone()))
        .unwrap_or_default()
}

pub async fn list_crds(client: &Client) -> AppResult<Vec<CrdSummary>> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let mut out: Vec<CrdSummary> = list
        .items
        .into_iter()
        .map(|crd| {
            let spec = crd.spec;
            let name = crd.metadata.name.clone().unwrap_or_default();
            let group = spec.group.clone();
            let kind = spec.names.kind.clone();
            let plural = spec.names.plural.clone();
            let short_names = spec.names.short_names.clone().unwrap_or_default();
            let scope = spec.scope.clone();
            let versions: Vec<String> = spec.versions.iter().map(|v| v.name.clone()).collect();
            let preferred = preferred_version(&spec.versions);
            let age = time::age_seconds(crd.metadata.creation_timestamp.as_ref());
            CrdSummary {
                name,
                group,
                kind,
                plural,
                short_names,
                scope,
                versions,
                preferred_version: preferred,
                age_seconds: age,
            }
        })
        .collect();
    out.sort_by(|a, b| a.group.cmp(&b.group).then(a.kind.cmp(&b.kind)));
    Ok(out)
}

fn api_resource(group: &str, version: &str, kind: &str, plural: &str) -> ApiResource {
    let gvk = GroupVersionKind::gvk(group, version, kind);
    ApiResource::from_gvk_with_plural(&gvk, plural)
}

fn status_hint_of(obj: &DynamicObject) -> Option<String> {
    let status = obj.data.get("status")?;
    // Prefer phase → conditions[ready] → just a compact key list.
    if let Some(phase) = status.get("phase").and_then(|v| v.as_str()) {
        return Some(format!("phase={phase}"));
    }
    if let Some(conds) = status.get("conditions").and_then(|v| v.as_array()) {
        if let Some(ready) = conds
            .iter()
            .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("Ready"))
        {
            let s = ready
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            return Some(format!("ready={s}"));
        }
    }
    let keys: Vec<&str> = status
        .as_object()
        .map(|o| o.keys().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    if keys.is_empty() {
        None
    } else {
        Some(keys.join(","))
    }
}

pub async fn list_instances(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    namespace: Option<String>,
) -> AppResult<Vec<CrInstance>> {
    let ar = api_resource(group, version, kind, plural);
    let api: Api<DynamicObject> = match namespace {
        Some(ref ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };
    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let mut out: Vec<CrInstance> = list
        .items
        .into_iter()
        .map(|obj| instance_of(&obj, &[]))
        .collect();
    out.sort_by(|a, b| {
        a.namespace
            .clone()
            .unwrap_or_default()
            .cmp(&b.namespace.clone().unwrap_or_default())
            .then(a.name.cmp(&b.name))
    });
    Ok(out)
}

pub async fn get_instance_yaml(
    client: &Client,
    group: &str,
    version: &str,
    kind: &str,
    plural: &str,
    namespace: Option<String>,
    name: &str,
) -> AppResult<String> {
    let ar = api_resource(group, version, kind, plural);
    let api: Api<DynamicObject> = match namespace {
        Some(ref ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    };
    let obj = api
        .get(name)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct CrTarget {
    pub crd_name: String,
    pub version: String,
    pub namespace: Option<String>,
    pub name: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrCondition {
    #[serde(rename = "type")]
    pub condition_type: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub observed_generation: Option<i64>,
    pub last_transition_time: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PrinterColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: String,
    pub description: Option<String>,
    pub json_path: String,
    pub priority: i32,
}
#[derive(Debug, Clone, Serialize)]
pub struct PrinterCell {
    pub name: String,
    pub value: Option<Value>,
    pub supported: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct CrVersion {
    pub name: String,
    pub served: bool,
    pub storage: bool,
    pub columns: Vec<PrinterColumn>,
    pub schema: Option<Value>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CrdDetails {
    pub name: String,
    pub group: String,
    pub kind: String,
    pub plural: String,
    pub scope: String,
    pub versions: Vec<CrVersion>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CrList {
    pub items: Vec<CrInstance>,
    pub columns: Vec<PrinterColumn>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CrDetail {
    pub yaml: String,
    pub uid: Option<String>,
    pub resource_version: Option<String>,
    pub generation: Option<i64>,
    pub conditions: Vec<CrCondition>,
}

pub async fn get_details(client: &Client, name: &str) -> AppResult<CrdDetails> {
    let crd = Api::<CustomResourceDefinition>::all(client.clone())
        .get(name)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    Ok(CrdDetails {
        name: crd.metadata.name.unwrap_or_default(),
        group: crd.spec.group,
        kind: crd.spec.names.kind,
        plural: crd.spec.names.plural,
        scope: crd.spec.scope,
        versions: crd
            .spec
            .versions
            .into_iter()
            .map(|v| CrVersion {
                name: v.name,
                served: v.served,
                storage: v.storage,
                schema: v
                    .schema
                    .and_then(|s| s.open_api_v3_schema)
                    .and_then(|s| serde_json::to_value(s).ok()),
                columns: v
                    .additional_printer_columns
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| PrinterColumn {
                        name: c.name,
                        column_type: c.type_,
                        description: c.description,
                        json_path: c.json_path,
                        priority: c.priority.unwrap_or(0),
                    })
                    .collect(),
            })
            .collect(),
    })
}

fn validate_target(details: &CrdDetails, target: &CrTarget, require_name: bool) -> AppResult<()> {
    if details.name != target.crd_name
        || !details
            .versions
            .iter()
            .any(|v| v.name == target.version && v.served)
    {
        return Err(AppError::Conflict(
            "CRD identity or served version changed; refresh the explorer".into(),
        ));
    }
    if !["Namespaced", "Cluster"].contains(&details.scope.as_str())
        || details.name != format!("{}.{}", details.plural, details.group)
    {
        return Err(AppError::Conflict(
            "Invalid CRD discovery identity or scope".into(),
        ));
    }
    // Reject path/query characters before constructing dynamic API URLs.
    let valid_segment = |value: &str, max: usize| {
        !value.is_empty()
            && value.len() <= max
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.')
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value.as_bytes()[value.len() - 1].is_ascii_alphanumeric()
    };
    if (!target.name.is_empty() && !valid_segment(&target.name, 253))
        || target
            .namespace
            .as_deref()
            .is_some_and(|ns| !valid_segment(ns, 63) || ns.contains('.'))
    {
        return Err(AppError::Conflict(
            "Invalid resource name or namespace".into(),
        ));
    }
    let ns = target.namespace.as_deref().filter(|s| !s.trim().is_empty());
    if details.scope == "Cluster" && target.namespace.is_some() {
        return Err(AppError::Conflict(
            "Cluster-scoped resources cannot specify a namespace".into(),
        ));
    }
    if require_name
        && (target.name.trim().is_empty() || (details.scope == "Namespaced" && ns.is_none()))
    {
        return Err(AppError::Conflict(
            "An explicit name and resource namespace are required".into(),
        ));
    }
    Ok(())
}
fn dynamic_api(client: &Client, details: &CrdDetails, target: &CrTarget) -> Api<DynamicObject> {
    let ar = api_resource(
        &details.group,
        &target.version,
        &details.kind,
        &details.plural,
    );
    match target.namespace.as_deref() {
        Some(ns) => Api::namespaced_with(client.clone(), ns, &ar),
        None => Api::all_with(client.clone(), &ar),
    }
}

// Deliberately bounded subset: dotted object fields only. Unsupported filters,
// indexes and escaped keys are visibly unavailable, never guessed.
fn printer_value(value: &Value, path: &str) -> (Option<Value>, bool) {
    let Some(path) = path.strip_prefix('.') else {
        return (None, false);
    };
    let fields: Vec<_> = path.split('.').collect();
    if fields.iter().any(|s| {
        s.is_empty()
            || !s
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }) {
        return (None, false);
    }
    let mut cursor = Some(value);
    for key in fields {
        cursor = cursor.and_then(|v| v.get(key));
    }
    (cursor.cloned(), true)
}
fn instance_of(obj: &DynamicObject, columns: &[PrinterColumn]) -> CrInstance {
    let value = serde_json::to_value(obj).unwrap_or(Value::Null);
    let conditions = obj
        .data
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let string = |key: &str| c.get(key).and_then(Value::as_str).map(str::to_owned);
                    CrCondition {
                        condition_type: string("type").unwrap_or_default(),
                        status: string("status").unwrap_or_else(|| "Unknown".into()),
                        reason: string("reason"),
                        message: string("message"),
                        observed_generation: c.get("observedGeneration").and_then(Value::as_i64),
                        last_transition_time: string("lastTransitionTime"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    CrInstance {
        name: obj.metadata.name.clone().unwrap_or_default(),
        namespace: obj.metadata.namespace.clone(),
        age_seconds: time::age_seconds(obj.metadata.creation_timestamp.as_ref()),
        status_hint: status_hint_of(obj),
        uid: obj.metadata.uid.clone(),
        resource_version: obj.metadata.resource_version.clone(),
        generation: obj.metadata.generation,
        conditions,
        printer_cells: columns
            .iter()
            .map(|c| {
                let (value, supported) = printer_value(&value, &c.json_path);
                PrinterCell {
                    name: c.name.clone(),
                    value,
                    supported,
                }
            })
            .collect(),
    }
}
pub async fn list_custom_resources(client: &Client, target: &CrTarget) -> AppResult<CrList> {
    let details = get_details(client, &target.crd_name).await?;
    validate_target(&details, target, false)?;
    let columns = details
        .versions
        .iter()
        .find(|v| v.name == target.version)
        .unwrap()
        .columns
        .clone();
    let list = dynamic_api(client, &details, target)
        .list(&ListParams::default())
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let mut items: Vec<_> = list
        .items
        .iter()
        .map(|o| instance_of(o, &columns))
        .collect();
    items.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    Ok(CrList { items, columns })
}
pub async fn get_custom_resource(client: &Client, target: &CrTarget) -> AppResult<CrDetail> {
    let details = get_details(client, &target.crd_name).await?;
    validate_target(&details, target, true)?;
    let obj = dynamic_api(client, &details, target)
        .get(&target.name)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let instance = instance_of(&obj, &[]);
    Ok(CrDetail {
        yaml: serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?,
        uid: instance.uid,
        resource_version: instance.resource_version,
        generation: instance.generation,
        conditions: instance.conditions,
    })
}

fn access_attributes(
    details: &CrdDetails,
    target: &CrTarget,
    verb: &str,
) -> AppResult<k8s_openapi::api::authorization::v1::ResourceAttributes> {
    if !["create", "patch", "delete"].contains(&verb) {
        return Err(AppError::PermissionDenied(
            "Unsupported custom-resource verb".into(),
        ));
    }
    Ok(k8s_openapi::api::authorization::v1::ResourceAttributes {
        group: Some(details.group.clone()),
        version: Some(target.version.clone()),
        resource: Some(details.plural.clone()),
        verb: Some(verb.into()),
        namespace: target.namespace.clone(),
        name: (verb != "create").then(|| target.name.clone()),
        ..Default::default()
    })
}
pub async fn check_access(
    client: &Client,
    target: &CrTarget,
    verb: &str,
) -> AppResult<crate::k8s::rbac::AccessReviewResult> {
    use k8s_openapi::api::authorization::v1::{
        SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
    };
    let details = get_details(client, &target.crd_name).await?;
    validate_target(&details, target, verb != "create")?;
    if details.scope == "Namespaced"
        && target
            .namespace
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(AppError::Conflict(
            "Select a namespace before checking mutation permissions".into(),
        ));
    }
    let review = SelfSubjectAccessReview {
        spec: SelfSubjectAccessReviewSpec {
            resource_attributes: Some(access_attributes(&details, target, verb)?),
            non_resource_attributes: None,
        },
        ..Default::default()
    };
    let response = Api::<SelfSubjectAccessReview>::all(client.clone())
        .create(&kube::api::PostParams::default(), &review)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let status = response.status.unwrap_or_default();
    Ok(crate::k8s::rbac::AccessReviewResult {
        allowed: status.allowed,
        denied: status.denied.unwrap_or(false),
        reason: status.reason,
        evaluation_error: status.evaluation_error,
    })
}
fn prepare_write(
    details: &CrdDetails,
    target: &CrTarget,
    yaml: &str,
    create: bool,
) -> AppResult<DynamicObject> {
    validate_target(details, target, true)?;
    let mut obj: DynamicObject = serde_yaml::from_str(yaml)
        .map_err(|e| AppError::Conflict(format!("Invalid resource YAML: {e}")))?;
    let types = obj
        .types
        .as_ref()
        .ok_or_else(|| AppError::Conflict("apiVersion and kind are required".into()))?;
    if types.api_version != format!("{}/{}", details.group, target.version)
        || types.kind != details.kind
        || obj.metadata.name.as_deref() != Some(&target.name)
        || obj.metadata.namespace != target.namespace
    {
        return Err(AppError::Conflict(
            "YAML identity must match the selected CRD, version, name, and namespace".into(),
        ));
    }
    if create {
        if obj.metadata.uid.is_some() || obj.metadata.resource_version.is_some() {
            return Err(AppError::Conflict(
                "New resources must not contain UID or resourceVersion".into(),
            ));
        }
    } else {
        if target.uid.as_deref().is_none_or(str::is_empty)
            || target.resource_version.as_deref().is_none_or(str::is_empty)
            || obj.metadata.uid != target.uid
            || obj.metadata.resource_version != target.resource_version
        {
            return Err(AppError::Conflict(
                "UID and resourceVersion must match the loaded resource; reload before editing"
                    .into(),
            ));
        }
    }
    obj.metadata.managed_fields = None;
    if let Some(data) = obj.data.as_object_mut() {
        data.remove("status");
    }
    Ok(obj)
}
pub async fn write_cr(
    client: &Client,
    target: &CrTarget,
    yaml: &str,
    create: bool,
    dry_run: bool,
    guard: impl FnOnce() -> AppResult<()>,
) -> AppResult<crate::k8s::actions::ApplyOutcome> {
    let details = get_details(client, &target.crd_name).await?;
    let obj = prepare_write(&details, target, yaml, create)?;
    let api = dynamic_api(client, &details, target);
    guard()?;
    let result = if create {
        api.create(
            &kube::api::PostParams {
                dry_run,
                field_manager: Some("lumen".into()),
            },
            &obj,
        )
        .await
    } else {
        // Include both UID and resourceVersion in the patch itself: a GET-only
        // precheck would permit a delete/recreate race or lost update.
        let mut params = kube::api::PatchParams::apply("lumen");
        params.dry_run = dry_run;
        api.patch(&target.name, &params, &kube::api::Patch::Apply(&obj))
            .await
    }
    .map_err(|e| AppError::K8s(e.to_string()))?;
    Ok(crate::k8s::actions::ApplyOutcome {
        yaml: serde_yaml::to_string(&result).map_err(|e| AppError::Internal(e.to_string()))?,
        dry_run,
    })
}
pub async fn delete_cr(
    client: &Client,
    target: &CrTarget,
    guard: impl FnOnce() -> AppResult<()>,
) -> AppResult<()> {
    let details = get_details(client, &target.crd_name).await?;
    validate_target(&details, target, true)?;
    if target.uid.as_deref().is_none_or(str::is_empty) {
        return Err(AppError::Conflict(
            "Reload the resource to obtain its UID before deletion".into(),
        ));
    }
    guard()?;
    dynamic_api(client, &details, target)
        .delete(
            &target.name,
            &kube::api::DeleteParams {
                preconditions: Some(kube::api::Preconditions {
                    uid: target.uid.clone(),
                    resource_version: target.resource_version.clone(),
                }),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn details() -> CrdDetails {
        CrdDetails {
            name: "widgets.example.io".into(),
            group: "example.io".into(),
            kind: "Widget".into(),
            plural: "widgets".into(),
            scope: "Namespaced".into(),
            versions: vec![CrVersion {
                name: "v1".into(),
                served: true,
                storage: true,
                columns: vec![],
                schema: None,
            }],
        }
    }
    fn target() -> CrTarget {
        CrTarget {
            crd_name: "widgets.example.io".into(),
            version: "v1".into(),
            namespace: Some("team".into()),
            name: "sample".into(),
            uid: Some("uid-1".into()),
            resource_version: Some("10".into()),
        }
    }
    fn document() -> Value {
        json!({"apiVersion":"example.io/v1","kind":"Widget","metadata":{"name":"sample","namespace":"team","uid":"uid-1","resourceVersion":"10","managedFields":[]},"spec":{"size":3},"status":{"phase":"Ready"}})
    }
    #[test]
    fn writes_pin_every_identity_field_and_optimistic_concurrency() {
        let details = details();
        let target = target();
        let value = document();
        let clean = prepare_write(&details, &target, &value.to_string(), false).unwrap();
        assert!(clean.data.get("status").is_none());
        assert!(clean.metadata.managed_fields.is_none());
        assert_eq!(clean.metadata.resource_version.as_deref(), Some("10"));
        assert_eq!(clean.metadata.uid.as_deref(), Some("uid-1"));
        for (pointer, replacement) in [
            ("/apiVersion", "other.io/v1"),
            ("/kind", "Secret"),
            ("/metadata/name", "other"),
            ("/metadata/namespace", "other"),
            ("/metadata/uid", "replacement"),
            ("/metadata/resourceVersion", "11"),
        ] {
            let mut bad = value.clone();
            *bad.pointer_mut(pointer).unwrap() = json!(replacement);
            assert!(
                prepare_write(&details, &target, &bad.to_string(), false).is_err(),
                "{pointer}"
            );
        }
        assert!(prepare_write(&details, &target, &value.to_string(), true).is_err());
    }
    #[test]
    fn discovery_rejects_unserved_versions_wrong_scope_and_missing_namespace() {
        let mut target = target();
        let mut details = details();
        target.version = "v2".into();
        assert!(validate_target(&details, &target, true).is_err());
        target.version = "v1".into();
        details.scope = "Cluster".into();
        assert!(validate_target(&details, &target, true).is_err());
        target.namespace = None;
        assert!(validate_target(&details, &target, true).is_ok());
        details.scope = "Namespaced".into();
        assert!(validate_target(&details, &target, true).is_err());
        assert!(validate_target(&details, &target, false).is_ok()); // all-namespace listing
    }
    #[test]
    fn printer_paths_distinguish_absent_values_from_unsupported_expressions() {
        let value = json!({"status":{"ready":false,"replicas":0}});
        assert_eq!(
            printer_value(&value, ".status.ready"),
            (Some(json!(false)), true)
        );
        assert_eq!(
            printer_value(&value, ".status.replicas"),
            (Some(json!(0)), true)
        );
        assert_eq!(printer_value(&value, ".status.absent"), (None, true));
        for path in [
            ".status.conditions[?(@.type=='Ready')].status",
            ".items[0]",
            ".metadata.labels.example\\.io/name",
            "",
        ] {
            assert_eq!(printer_value(&value, path), (None, false));
        }
    }
    #[test]
    fn access_uses_actual_group_plural_and_unnamed_create() {
        let details = details();
        let target = target();
        let attrs = access_attributes(&details, &target, "create").unwrap();
        assert_eq!(attrs.group.as_deref(), Some("example.io"));
        assert_eq!(attrs.resource.as_deref(), Some("widgets"));
        assert!(attrs.name.is_none());
        assert_eq!(
            access_attributes(&details, &target, "patch")
                .unwrap()
                .name
                .as_deref(),
            Some("sample")
        );
        assert!(access_attributes(&details, &target, "deletecollection").is_err());
    }
    async fn mock_client() -> (wiremock::MockServer, Client) {
        let server = wiremock::MockServer::start().await;
        let config = kube::Config::new(server.uri().parse().unwrap());
        (server, Client::try_from(config).unwrap())
    }
    fn crd_document() -> Value {
        json!({"apiVersion":"apiextensions.k8s.io/v1","kind":"CustomResourceDefinition","metadata":{"name":"widgets.example.io"},"spec":{"group":"example.io","names":{"kind":"Widget","plural":"widgets"},"scope":"Namespaced","versions":[{"name":"v1","served":true,"storage":true,"schema":{"openAPIV3Schema":{"type":"object"}}}]}})
    }
    async fn mock_discovery(server: &wiremock::MockServer) {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/apis/apiextensions.k8s.io/v1/customresourcedefinitions/widgets.example.io",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(crd_document()))
            .mount(server)
            .await;
    }
    #[tokio::test]
    async fn create_dry_run_posts_and_update_uses_nonforce_ssa_with_identity_guards() {
        use wiremock::{
            matchers::{body_partial_json, method, path, query_param},
            Mock, ResponseTemplate,
        };
        let (server, client) = mock_client().await;
        mock_discovery(&server).await;
        let mut value = document();
        value["metadata"].as_object_mut().unwrap().remove("uid");
        value["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("resourceVersion");
        Mock::given(method("POST"))
            .and(path("/apis/example.io/v1/namespaces/team/widgets"))
            .and(query_param("dryRun", "All"))
            .respond_with(ResponseTemplate::new(201).set_body_json(value.clone()))
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            write_cr(
                &client,
                &target(),
                &value.to_string(),
                true,
                true,
                || Ok(())
            )
            .await
            .unwrap()
            .dry_run
        );
        Mock::given(method("PATCH"))
            .and(path("/apis/example.io/v1/namespaces/team/widgets/sample"))
            .and(query_param("dryRun", "All"))
            .and(body_partial_json(
                json!({"metadata":{"uid":"uid-1","resourceVersion":"10"}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(document()))
            .expect(1)
            .mount(&server)
            .await;
        write_cr(
            &client,
            &target(),
            &document().to_string(),
            false,
            true,
            || Ok(()),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        let patch = requests.iter().find(|r| r.method == "PATCH").unwrap();
        assert!(!patch
            .url
            .query_pairs()
            .any(|(k, v)| k == "force" && v == "true"));
        let body: Value = serde_json::from_slice(&patch.body).unwrap();
        assert!(body.get("status").is_none());
        server.verify().await;
    }
    #[tokio::test]
    async fn invalid_identity_never_reaches_mutation_endpoint_and_delete_has_uid_precondition() {
        use wiremock::{
            matchers::{body_partial_json, method, path},
            Mock, ResponseTemplate,
        };
        let (server, client) = mock_client().await;
        mock_discovery(&server).await;
        let mut bad = document();
        bad["kind"] = json!("Secret");
        assert!(write_cr(
            &client,
            &target(),
            &bad.to_string(),
            false,
            false,
            || Ok(())
        )
        .await
        .is_err());
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"));
        Mock::given(method("DELETE"))
            .and(path("/apis/example.io/v1/namespaces/team/widgets/sample"))
            .and(body_partial_json(
                json!({"preconditions":{"uid":"uid-1","resourceVersion":"10"}}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(document()))
            .expect(1)
            .mount(&server)
            .await;
        delete_cr(&client, &target(), || Ok(())).await.unwrap();
        server.verify().await;
    }
    #[tokio::test]
    async fn rbac_preflight_targets_discovered_resource() {
        use wiremock::{
            matchers::{body_partial_json, method, path},
            Mock, ResponseTemplate,
        };
        let (server, client) = mock_client().await;
        mock_discovery(&server).await;
        Mock::given(method("POST")).and(path("/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"))
            .and(body_partial_json(json!({"spec":{"resourceAttributes":{"group":"example.io","resource":"widgets","verb":"create","namespace":"team"}}})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectAccessReview","spec":{},"status":{"allowed":false,"reason":"denied"}}))).expect(1).mount(&server).await;
        assert!(
            !check_access(&client, &target(), "create")
                .await
                .unwrap()
                .allowed
        );
        server.verify().await;
        let requests = server.received_requests().await.unwrap();
        let request = requests.iter().find(|r| r.method == "POST").unwrap();
        let value: Value = serde_json::from_slice(&request.body).unwrap();
        assert!(value.pointer("/spec/resourceAttributes/name").is_none());
    }
    #[tokio::test]
    async fn revoked_guard_after_discovery_blocks_write_and_delete() {
        let (server, client) = mock_client().await;
        mock_discovery(&server).await;
        let deny = || Err(AppError::PermissionDenied("unlock expired".into()));
        assert!(matches!(
            write_cr(
                &client,
                &target(),
                &document().to_string(),
                false,
                false,
                deny
            )
            .await,
            Err(AppError::PermissionDenied(_))
        ));
        assert!(matches!(
            delete_cr(&client, &target(), deny).await,
            Err(AppError::PermissionDenied(_))
        ));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r.method == "GET"));
    }
    #[tokio::test]
    async fn existing_create_and_stale_update_conflicts_are_not_retried_or_forced() {
        use wiremock::{
            matchers::{method, path},
            Mock, ResponseTemplate,
        };
        let (server, client) = mock_client().await;
        mock_discovery(&server).await;
        let conflict = json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"Conflict","message":"object already exists or resource version changed","code":409});
        Mock::given(method("PATCH"))
            .and(path("/apis/example.io/v1/namespaces/team/widgets/sample"))
            .respond_with(ResponseTemplate::new(409).set_body_json(conflict.clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apis/example.io/v1/namespaces/team/widgets"))
            .respond_with(ResponseTemplate::new(409).set_body_json(conflict))
            .expect(1)
            .mount(&server)
            .await;
        assert!(write_cr(
            &client,
            &target(),
            &document().to_string(),
            false,
            false,
            || Ok(())
        )
        .await
        .is_err());
        let mut value = document();
        value["metadata"].as_object_mut().unwrap().remove("uid");
        value["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("resourceVersion");
        assert!(
            write_cr(&client, &target(), &value.to_string(), true, false, || Ok(
                ()
            ))
            .await
            .is_err()
        );
        server.verify().await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.iter().filter(|r| r.method != "GET").count(), 2);
    }
    #[test]
    fn target_rejects_path_injection_and_unrecognized_scope() {
        let mut target = target();
        let mut details = details();
        target.name = "sample/status".into();
        assert!(validate_target(&details, &target, true).is_err());
        target.name = "sample?dryRun=All".into();
        assert!(validate_target(&details, &target, true).is_err());
        target.name = "sample".into();
        target.namespace = Some("team/other".into());
        assert!(validate_target(&details, &target, true).is_err());
        target.namespace = Some("team".into());
        details.scope = "Unknown".into();
        assert!(validate_target(&details, &target, true).is_err());
    }
}
