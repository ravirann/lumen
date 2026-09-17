use crate::error::{AppError, AppResult};
use crate::k8s::{
    actions as act, argocd, cloudmap, crd as crd_mod, fleet, kubeconfig, metrics, rbac, rbac_admin,
    rbac_details, registry, resource_insights, resources, security, storage_details, tekton, time,
    types::{
        CloudMap, ContainerInfo, ContextInfo, FleetCard, IntOrStringValue, NetworkDebugSnapshot,
        NetworkEndpointAddress, NetworkEndpointPort, NetworkEndpointRef, NetworkEndpointSlice,
        NetworkEndpoints, NetworkIngress, NetworkIngressBackend, NetworkIngressRule,
        NetworkNamespace, NetworkPod, NetworkPodPort, NetworkPolicyEgressRule,
        NetworkPolicyIngressRule, NetworkPolicyIpBlock, NetworkPolicyPeer, NetworkPolicyPort,
        NetworkPolicyResource, NetworkService, NetworkServicePort, NodeSummary, OwnerRefLite,
        PodCondition, PodDetails, RbacDetail, ResourceDetail, ResourceInsights, SecurityReport,
        StorageDetail, WorkloadKind, WorkloadSummary,
    },
};
use crate::state::AppState;
use k8s_openapi::api::{
    admissionregistration::v1::{MutatingWebhookConfiguration, ValidatingWebhookConfiguration},
    apps::v1::{DaemonSet, Deployment, StatefulSet},
    autoscaling::v2::HorizontalPodAutoscaler,
    batch::v1::{CronJob, Job},
    core::v1::{
        ConfigMap, Endpoints, LimitRange, Namespace, PersistentVolume, PersistentVolumeClaim, Pod,
        ResourceQuota, Secret, Service,
    },
    discovery::v1::EndpointSlice,
    networking::v1::{Ingress, IngressClass, NetworkPolicy},
    policy::v1::PodDisruptionBudget,
    rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding},
    scheduling::v1::PriorityClass,
    storage::v1::StorageClass,
};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    api::{DynamicObject, ListParams},
    core::{ApiResource, GroupVersionKind},
    Api,
};
use tauri::{AppHandle, State};

fn owner_refs_from(
    meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
) -> Vec<OwnerRefLite> {
    meta.owner_references
        .as_ref()
        .map(|v| {
            v.iter()
                .map(|o| OwnerRefLite {
                    kind: o.kind.clone(),
                    name: o.name.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve a kube::Client from either an explicit context (new multi-cluster
/// flows) or the active context (legacy single-cluster flows).
pub(super) async fn client_for(state: &AppState, context: Option<&str>) -> AppResult<kube::Client> {
    let ctx = state.k8s.resolve_context(context).await?;
    state.k8s.client_for(&ctx).await
}

fn api_resource_for(definition: &registry::ResourceDefinition) -> ApiResource {
    let gvk = GroupVersionKind::gvk(
        definition.api_group,
        definition.version,
        registry::api_kind(&definition.kind),
    );
    ApiResource::from_gvk_with_plural(&gvk, definition.plural)
}

fn is_not_found(err: &kube::Error) -> bool {
    matches!(err, kube::Error::Api(api_err) if api_err.code == 404)
}

async fn list_dynamic_resources(
    client: kube::Client,
    namespace: &str,
    kind: &WorkloadKind,
) -> AppResult<Vec<WorkloadSummary>> {
    let definition = registry::get_resource_definition(kind)
        .ok_or_else(|| AppError::Internal(format!("resource kind {kind:?} is not registered")))?;
    let ar = api_resource_for(definition);
    let api: Api<DynamicObject> = if definition.namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::all_with(client, &ar)
    };
    let list = match api.list(&ListParams::default()).await {
        Ok(list) => list,
        Err(err) if definition.optional && is_not_found(&err) => return Ok(vec![]),
        Err(err) => return Err(AppError::K8s(err.to_string())),
    };

    Ok(list
        .items
        .iter()
        .map(|obj| resources::dynamic_summary(obj, definition))
        .collect())
}

async fn get_dynamic_resource(
    client: kube::Client,
    namespace: &str,
    kind: &WorkloadKind,
    name: &str,
) -> AppResult<ResourceDetail> {
    let definition = registry::get_resource_definition(kind)
        .ok_or_else(|| AppError::Internal(format!("resource kind {kind:?} is not registered")))?;
    let ar = api_resource_for(definition);
    let api: Api<DynamicObject> = if definition.namespaced && !namespace.is_empty() {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::all_with(client, &ar)
    };
    let obj = api
        .get(name)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let summary = resources::dynamic_summary(&obj, definition);
    let yaml = serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
    let owner_refs = owner_refs_from(&obj.metadata);
    Ok(ResourceDetail {
        summary,
        yaml,
        owner_refs,
    })
}

// ─── Contexts & namespaces ────────────────────────────────────────────────

// Mutation IPCs never fall back to a globally selected context. The policy and
// client share one effective kubeconfig snapshot, even if files change later.
async fn mutation_target(
    state: &AppState,
    context: Option<&str>,
    dry_run: bool,
) -> AppResult<(String, kube::config::Kubeconfig, String)> {
    let context = context
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            crate::error::AppError::PermissionDenied(
                "An explicit context is required for mutations".into(),
            )
        })?
        .to_owned();
    let (config, identity) = crate::protection::load_target(&context).inspect_err(|_| {
        let _ = state.protection.lock(&context, "");
    })?;
    state
        .protection
        .require_mutation(&context, &identity, dry_run)?;
    Ok((context, config, identity))
}

pub(super) async fn mutation_client(
    state: &AppState,
    context: Option<&str>,
    dry_run: bool,
) -> AppResult<(String, kube::Client, String, kube::config::Kubeconfig)> {
    let (context, config, identity) = mutation_target(state, context, dry_run).await?;
    let target = mutation_client_from_config(state, context, config, identity, dry_run).await?;
    require_current_target(&state.protection, &target.0, &target.2, dry_run)?;
    Ok(target)
}

async fn mutation_client_from_config(
    state: &AppState,
    context: String,
    config: kube::config::Kubeconfig,
    identity: String,
    dry_run: bool,
) -> AppResult<(String, kube::Client, String, kube::config::Kubeconfig)> {
    state
        .protection
        .require_mutation(&context, &identity, dry_run)?;
    let options = kube::config::KubeConfigOptions {
        context: Some(context.clone()),
        ..Default::default()
    };
    let client_config = kube::Config::from_custom_kubeconfig(config.clone(), &options)
        .await
        .map_err(|e| crate::error::AppError::Kubeconfig(e.to_string()))?;
    let client = kube::Client::try_from(client_config)
        .map_err(|e| crate::error::AppError::K8s(e.to_string()))?;
    // Client setup may invoke an authentication helper. Recheck expiry/lock
    // immediately before returning the client to the mutating operation.
    state
        .protection
        .require_mutation(&context, &identity, dry_run)?;
    Ok((context, client, identity, config))
}

pub(super) fn require_current_target(
    policy: &crate::protection::ContextProtectionPolicy,
    context: &str,
    identity: &str,
    dry_run: bool,
) -> AppResult<()> {
    let current = context_identity(context).inspect_err(|_| {
        let _ = policy.lock(context, "");
    })?;
    if current != identity {
        let _ = policy.lock(context, &current);
        return Err(AppError::PermissionDenied("Context configuration changed before the operation started. Review the context and try again.".into()));
    }
    policy.require_mutation(context, identity, dry_run)
}

fn context_identity(context: &str) -> AppResult<String> {
    crate::protection::load_target(context).map(|(_, identity)| identity)
}

#[tauri::command]
pub async fn get_context_protection(
    context: String,
    state: State<'_, AppState>,
) -> AppResult<crate::protection::ContextProtection> {
    let identity = context_identity(&context).inspect_err(|_| {
        let _ = state.protection.lock(&context, "");
    })?;
    state.protection.status(&context, &identity)
}
#[tauri::command]
pub async fn set_context_protection(
    context: String,
    protected: bool,
    state: State<'_, AppState>,
) -> AppResult<crate::protection::ContextProtection> {
    let identity = context_identity(&context)?;
    let result = state
        .protection
        .set_protected(&context, protected, &identity);
    state.attachments.close_context(&context).await;
    result
}
#[tauri::command]
pub async fn unlock_context(
    context: String,
    state: State<'_, AppState>,
) -> AppResult<crate::protection::ContextProtection> {
    state
        .protection
        .unlock(&context, &context_identity(&context)?)
}
#[tauri::command]
pub async fn lock_context(
    context: String,
    state: State<'_, AppState>,
) -> AppResult<crate::protection::ContextProtection> {
    // Close sessions even if kubeconfig was removed/corrupted since they opened.
    state.attachments.close_context(&context).await;
    state
        .protection
        .lock(&context, &context_identity(&context).unwrap_or_default())
}

#[tauri::command]
pub async fn list_contexts() -> AppResult<Vec<ContextInfo>> {
    let kc = kubeconfig::load()?;
    Ok(kubeconfig::list_contexts_from(&kc))
}

#[tauri::command]
pub async fn set_context(name: String, state: State<'_, AppState>) -> AppResult<ContextInfo> {
    state.k8s.set_context(&name).await?;
    let kc = kubeconfig::load()?;
    let ctxs = kubeconfig::list_contexts_from(&kc);
    ctxs.into_iter()
        .find(|c| c.name == name)
        .ok_or_else(|| AppError::Kubeconfig(format!("context '{name}' not found after switch")))
}

#[tauri::command]
pub async fn delete_context(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    kubeconfig::delete_context(&app, &name)?;
    let _ = state.protection.lock(&name, "");
    state.attachments.close_context(&name).await;
    state.k8s.invalidate(&name).await;
    metrics::invalidate_context(&name);
    Ok(())
}

#[tauri::command]
pub async fn list_deleted_contexts(
    app: AppHandle,
) -> AppResult<Vec<kubeconfig::DeletedContextSummary>> {
    kubeconfig::list_deleted_contexts(&app)
}

#[tauri::command]
pub async fn restore_deleted_context(
    name: String,
    overwrite: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<()> {
    kubeconfig::restore_deleted_context(&app, &name, overwrite)?;
    let _ = state.protection.lock(&name, "");
    state.attachments.close_context(&name).await;
    state.k8s.invalidate(&name).await;
    metrics::invalidate_context(&name);
    Ok(())
}

#[tauri::command]
pub async fn list_namespaces(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<String>> {
    let client = client_for(&state, context.as_deref()).await?;
    let api: Api<Namespace> = Api::all(client);
    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    Ok(list
        .items
        .into_iter()
        .filter_map(|n| n.metadata.name)
        .collect())
}

// ─── Fleet ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_fleet(state: State<'_, AppState>) -> AppResult<Vec<FleetCard>> {
    fleet::probe_all(&state.k8s).await
}

#[tauri::command]
pub async fn probe_fleet_context(
    context: String,
    state: State<'_, AppState>,
) -> AppResult<FleetCard> {
    fleet::probe_context(&state.k8s, &context).await
}

#[tauri::command]
pub async fn disconnect_context(context: String, state: State<'_, AppState>) -> AppResult<()> {
    state.k8s.invalidate(&context).await;
    metrics::invalidate_context(&context);
    Ok(())
}

/// Evict every cached client so the next call rebuilds fresh connections.
/// Useful when a private network route just came up or kubeconfig changed on disk.
#[tauri::command]
pub async fn reconnect_all(state: State<'_, AppState>) -> AppResult<()> {
    state.k8s.invalidate_all().await;
    metrics::invalidate_all();
    Ok(())
}

#[tauri::command]
pub async fn list_nodes(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<NodeSummary>> {
    let ctx = state.k8s.resolve_context(context.as_deref()).await?;
    let client = state.k8s.client_for(&ctx).await?;
    fleet::list_nodes(&client, &ctx).await
}

#[tauri::command]
pub async fn metrics_explorer_snapshot(
    namespace: Option<String>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<crate::k8s::metrics_explorer::MetricsExplorerSnapshot> {
    let ctx = state.k8s.resolve_context(context.as_deref()).await?;
    let client = state.k8s.client_for(&ctx).await?;
    crate::k8s::metrics_explorer::snapshot(
        &client,
        &ctx,
        namespace.as_deref().filter(|value| !value.is_empty()),
    )
    .await
}

// ─── CloudMap & Security ──────────────────────────────────────────────────

#[tauri::command]
pub async fn cloud_map(
    context: Option<String>,
    namespace: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<CloudMap> {
    let ctx = state.k8s.resolve_context(context.as_deref()).await?;
    let client = state.k8s.client_for(&ctx).await?;
    let mut m = cloudmap::build(&client, &ctx, namespace).await?;
    m.context = ctx;
    Ok(m)
}

#[tauri::command]
pub async fn network_debug_snapshot(
    namespace: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<NetworkDebugSnapshot> {
    let client = client_for(&state, context.as_deref()).await?;
    network_debug_snapshot_for(client, namespace).await
}

async fn network_debug_snapshot_for(
    client: kube::Client,
    namespace: String,
) -> AppResult<NetworkDebugSnapshot> {
    let lp = ListParams::default();
    let ns_api: Api<Namespace> = Api::all(client.clone());
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let service_api: Api<Service> = Api::namespaced(client.clone(), &namespace);
    let endpoints_api: Api<Endpoints> = Api::namespaced(client.clone(), &namespace);
    let endpoint_slice_api: Api<EndpointSlice> = Api::namespaced(client.clone(), &namespace);
    let ingress_api: Api<Ingress> = Api::namespaced(client.clone(), &namespace);
    let network_policy_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);

    let (namespaces, pods, services, endpoints, endpoint_slices, ingresses, network_policies) = tokio::join!(
        ns_api.list(&lp),
        pod_api.list(&lp),
        service_api.list(&lp),
        endpoints_api.list(&lp),
        endpoint_slice_api.list(&lp),
        ingress_api.list(&lp),
        network_policy_api.list(&lp),
    );
    let mut unavailable = std::collections::BTreeMap::new();
    macro_rules! evidence {
        ($result:ident, $key:expr) => {
            let $result = match $result {
                Ok(list) => list.items,
                Err(error) => {
                    unavailable.insert(format!("{}/{}", namespace, $key), error.to_string());
                    vec![]
                }
            };
        };
    }
    evidence!(namespaces, "namespaces");
    evidence!(pods, "pods");
    evidence!(services, "services");
    evidence!(endpoints, "endpoints");
    evidence!(endpoint_slices, "endpointSlices");
    evidence!(ingresses, "ingresses");
    evidence!(network_policies, "networkPolicies");
    let mut gateway_resources = vec![];
    for (kind, plural, version) in [
        ("Gateway", "gateways", "v1"),
        ("HTTPRoute", "httproutes", "v1"),
        ("ReferenceGrant", "referencegrants", "v1beta1"),
    ] {
        let ar = ApiResource::from_gvk_with_plural(
            &GroupVersionKind::gvk("gateway.networking.k8s.io", version, kind),
            plural,
        );
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), &namespace, &ar);
        match api.list(&lp).await {
            Ok(list) => gateway_resources.extend(
                list.items
                    .into_iter()
                    .filter_map(|obj| serde_json::to_value(obj).ok()),
            ),
            Err(error) => {
                unavailable.insert(format!("{}/{}", namespace, plural), error.to_string());
            }
        }
    }
    Ok(NetworkDebugSnapshot {
        loaded_namespaces: vec![namespace],
        unavailable,
        gateway_resources,
        namespaces: namespaces.iter().map(network_namespace).collect(),
        pods: pods.iter().map(network_pod).collect(),
        services: services.iter().map(network_service).collect(),
        endpoints: endpoints.iter().map(network_endpoints).collect(),
        endpoint_slices: endpoint_slices.iter().map(network_endpoint_slice).collect(),
        ingresses: ingresses.iter().map(network_ingress).collect(),
        network_policies: network_policies
            .iter()
            .map(network_policy_resource)
            .collect(),
    })
}

fn labels_from(
    labels: &Option<std::collections::BTreeMap<String, String>>,
) -> std::collections::BTreeMap<String, String> {
    labels.clone().unwrap_or_default()
}

fn network_namespace(ns: &Namespace) -> NetworkNamespace {
    NetworkNamespace {
        name: ns.metadata.name.clone().unwrap_or_default(),
        labels: labels_from(&ns.metadata.labels),
    }
}

fn network_pod(pod: &Pod) -> NetworkPod {
    let phase = pod.status.as_ref().and_then(|s| s.phase.clone());
    let ready = pod
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|condition| condition.type_ == "Ready")
        })
        .map(|condition| condition.status == "True")
        .unwrap_or_else(|| phase.as_deref() == Some("Running"));
    let ports = pod
        .spec
        .as_ref()
        .map(|spec| {
            spec.containers
                .iter()
                .flat_map(|container| container.ports.clone().unwrap_or_default())
                .map(|port| NetworkPodPort {
                    name: port.name,
                    container_port: port.container_port,
                    protocol: port.protocol,
                })
                .collect()
        })
        .unwrap_or_default();
    NetworkPod {
        name: pod.metadata.name.clone().unwrap_or_default(),
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        labels: labels_from(&pod.metadata.labels),
        ready,
        phase,
        pod_ip: pod.status.as_ref().and_then(|s| s.pod_ip.clone()),
        ports,
    }
}

fn int_or_string_value(value: &IntOrString) -> IntOrStringValue {
    match value {
        IntOrString::Int(i) => IntOrStringValue::Int(*i),
        IntOrString::String(s) => IntOrStringValue::String(s.clone()),
    }
}

fn network_service(service: &Service) -> NetworkService {
    let spec = service.spec.as_ref();
    let selector = spec
        .and_then(|s| s.selector.clone())
        .map(|labels| labels.into_iter().collect())
        .unwrap_or_default();
    let ports = spec
        .and_then(|s| s.ports.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|port| NetworkServicePort {
            name: port.name,
            protocol: port.protocol,
            port: port.port,
            target_port: port.target_port.as_ref().map(int_or_string_value),
        })
        .collect();
    NetworkService {
        name: service.metadata.name.clone().unwrap_or_default(),
        namespace: service.metadata.namespace.clone().unwrap_or_default(),
        r#type: spec.and_then(|s| s.type_.clone()),
        selector,
        ports,
    }
}

fn network_endpoint_ref(
    target: &Option<k8s_openapi::api::core::v1::ObjectReference>,
) -> Option<NetworkEndpointRef> {
    target.as_ref().map(|target| NetworkEndpointRef {
        kind: target.kind.clone(),
        name: target.name.clone(),
        namespace: target.namespace.clone(),
    })
}

fn network_endpoints(endpoints: &Endpoints) -> NetworkEndpoints {
    let mut addresses = Vec::new();
    for subset in endpoints.subsets.clone().unwrap_or_default() {
        for address in subset.addresses.unwrap_or_default() {
            addresses.push(NetworkEndpointAddress {
                addresses: vec![address.ip],
                ready: true,
                target_ref: network_endpoint_ref(&address.target_ref),
            });
        }
        for address in subset.not_ready_addresses.unwrap_or_default() {
            addresses.push(NetworkEndpointAddress {
                addresses: vec![address.ip],
                ready: false,
                target_ref: network_endpoint_ref(&address.target_ref),
            });
        }
    }
    NetworkEndpoints {
        name: endpoints.metadata.name.clone().unwrap_or_default(),
        namespace: endpoints.metadata.namespace.clone().unwrap_or_default(),
        addresses,
    }
}

fn network_endpoint_slice(slice: &EndpointSlice) -> NetworkEndpointSlice {
    let service_name = slice
        .metadata
        .labels
        .as_ref()
        .and_then(|labels| labels.get("kubernetes.io/service-name").cloned())
        .unwrap_or_else(|| slice.metadata.name.clone().unwrap_or_default());
    NetworkEndpointSlice {
        name: slice.metadata.name.clone().unwrap_or_default(),
        namespace: slice.metadata.namespace.clone().unwrap_or_default(),
        service_name,
        ports: slice
            .ports
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|port| NetworkEndpointPort {
                name: port.name,
                protocol: port.protocol,
                port: port.port,
            })
            .collect(),
        endpoints: slice
            .endpoints
            .iter()
            .map(|endpoint| NetworkEndpointAddress {
                addresses: endpoint.addresses.clone(),
                ready: endpoint
                    .conditions
                    .as_ref()
                    .and_then(|conditions| conditions.ready)
                    .unwrap_or(true),
                target_ref: network_endpoint_ref(&endpoint.target_ref),
            })
            .collect(),
    }
}

fn network_ingress(ingress: &Ingress) -> NetworkIngress {
    let spec = ingress.spec.as_ref();
    let mut rules: Vec<NetworkIngressRule> = spec
        .and_then(|spec| spec.rules.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|rule| NetworkIngressRule {
            host: rule.host,
            paths: rule
                .http
                .map(|http| {
                    http.paths
                        .into_iter()
                        .filter_map(|path| {
                            let service = path.backend.service?;
                            let service_port = service.port.as_ref().and_then(ingress_service_port);
                            Some(NetworkIngressBackend {
                                path: path.path.unwrap_or_else(|| "/".into()),
                                path_type: Some(path.path_type),
                                service_name: service.name,
                                service_port,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    if let Some(default_service) = spec
        .and_then(|spec| spec.default_backend.clone())
        .and_then(|backend| backend.service)
    {
        let service_port = default_service.port.as_ref().and_then(ingress_service_port);
        rules.push(NetworkIngressRule {
            host: None,
            paths: vec![NetworkIngressBackend {
                path: "/".into(),
                path_type: Some("ImplementationSpecific".into()),
                service_name: default_service.name,
                service_port,
            }],
        });
    }
    NetworkIngress {
        name: ingress.metadata.name.clone().unwrap_or_default(),
        namespace: ingress.metadata.namespace.clone().unwrap_or_default(),
        class_name: spec.and_then(|spec| spec.ingress_class_name.clone()),
        rules,
    }
}

fn ingress_service_port(
    port: &k8s_openapi::api::networking::v1::ServiceBackendPort,
) -> Option<IntOrStringValue> {
    port.name
        .clone()
        .map(IntOrStringValue::String)
        .or_else(|| port.number.map(IntOrStringValue::Int))
}

fn label_selector_match_labels(
    selector: &Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector>,
) -> Option<std::collections::BTreeMap<String, String>> {
    selector
        .as_ref()
        .map(|selector| selector.match_labels.clone().unwrap_or_default())
        .map(|labels| labels.into_iter().collect())
}

fn network_policy_peer(
    peer: k8s_openapi::api::networking::v1::NetworkPolicyPeer,
) -> NetworkPolicyPeer {
    NetworkPolicyPeer {
        unsupported: [&peer.pod_selector, &peer.namespace_selector]
            .iter()
            .any(|selector| {
                selector
                    .as_ref()
                    .is_some_and(|s| s.match_expressions.as_ref().is_some_and(|e| !e.is_empty()))
            }),
        pod_selector: label_selector_match_labels(&peer.pod_selector),
        namespace_selector: label_selector_match_labels(&peer.namespace_selector),
        ip_block: peer.ip_block.map(|block| NetworkPolicyIpBlock {
            cidr: block.cidr,
            except: block.except.unwrap_or_default(),
        }),
    }
}

fn network_policy_port(
    port: k8s_openapi::api::networking::v1::NetworkPolicyPort,
) -> NetworkPolicyPort {
    NetworkPolicyPort {
        end_port: port.end_port,
        protocol: port.protocol,
        port: port.port.as_ref().map(int_or_string_value),
    }
}

fn network_policy_resource(policy: &NetworkPolicy) -> NetworkPolicyResource {
    let spec = policy.spec.as_ref();
    NetworkPolicyResource {
        unsupported: spec
            .and_then(|s| s.pod_selector.as_ref())
            .is_some_and(|s| s.match_expressions.as_ref().is_some_and(|e| !e.is_empty())),
        egress: spec.and_then(|s| s.egress.clone()).map(|rules| {
            rules
                .into_iter()
                .map(|rule| NetworkPolicyEgressRule {
                    to: rule
                        .to
                        .unwrap_or_default()
                        .into_iter()
                        .map(network_policy_peer)
                        .collect(),
                    ports: rule
                        .ports
                        .unwrap_or_default()
                        .into_iter()
                        .map(network_policy_port)
                        .collect(),
                })
                .collect()
        }),
        name: policy.metadata.name.clone().unwrap_or_default(),
        namespace: policy.metadata.namespace.clone().unwrap_or_default(),
        pod_selector: spec
            .and_then(|spec| spec.pod_selector.as_ref())
            .and_then(|selector| selector.match_labels.clone())
            .map(|labels| labels.into_iter().collect())
            .unwrap_or_default(),
        policy_types: spec
            .and_then(|spec| spec.policy_types.clone())
            .unwrap_or_default(),
        ingress: spec
            .and_then(|spec| spec.ingress.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|rule| NetworkPolicyIngressRule {
                from: rule
                    .from
                    .unwrap_or_default()
                    .into_iter()
                    .map(network_policy_peer)
                    .collect(),
                ports: rule
                    .ports
                    .unwrap_or_default()
                    .into_iter()
                    .map(network_policy_port)
                    .collect(),
            })
            .collect(),
    }
}

#[tauri::command]
pub async fn security_scan(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<SecurityReport> {
    let ctx = state.k8s.resolve_context(context.as_deref()).await?;
    let client = state.k8s.client_for(&ctx).await?;
    security::scan(&client, ctx).await
}

#[tauri::command]
pub async fn check_access(
    request: rbac::AccessReviewRequest,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<rbac::AccessReviewResult> {
    let client = client_for(&state, context.as_deref()).await?;
    rbac::check_access(&client, request).await
}

// ─── Workloads ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_workloads(
    namespace: String,
    kind: WorkloadKind,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<WorkloadSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    let lp = ListParams::default();
    let summaries: Vec<WorkloadSummary> = match kind {
        WorkloadKind::Deployment => {
            let api: Api<Deployment> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::deployment_summary)
                .collect()
        }
        WorkloadKind::StatefulSet => {
            let api: Api<StatefulSet> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::statefulset_summary)
                .collect()
        }
        WorkloadKind::DaemonSet => {
            let api: Api<DaemonSet> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::daemonset_summary)
                .collect()
        }
        WorkloadKind::CronJob => {
            let api: Api<CronJob> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::cronjob_summary)
                .collect()
        }
        WorkloadKind::Job => {
            let api: Api<Job> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::job_summary)
                .collect()
        }
        WorkloadKind::Pod => {
            let api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
            let mut summaries: Vec<WorkloadSummary> = api
                .list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::pod_summary)
                .collect();
            // Fold metrics-server data (cached, ~free) into pod rows so the
            // table can show CPU/Memory without N+1 frontend round-trips.
            // Skip silently when metrics-server is unavailable.
            let ctx_name = context.as_deref().unwrap_or("");
            if !ctx_name.is_empty() {
                if let Some(usage) = crate::k8s::metrics::pod_usage(&client, ctx_name).await {
                    let mut by_key: std::collections::HashMap<(String, String), (i64, i64)> =
                        std::collections::HashMap::with_capacity(usage.len());
                    for u in usage {
                        by_key.insert((u.namespace, u.name), (u.cpu_milli, u.mem_bytes));
                    }
                    for s in summaries.iter_mut() {
                        if let Some((cpu, mem)) = by_key.get(&(s.namespace.clone(), s.name.clone()))
                        {
                            s.cpu_milli = Some(*cpu);
                            s.mem_bytes = Some(*mem);
                        }
                    }
                }
            }
            summaries
        }
        WorkloadKind::Service => {
            let api: Api<Service> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::service_summary)
                .collect()
        }
        WorkloadKind::Ingress => {
            let api: Api<Ingress> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::ingress_summary)
                .collect()
        }
        WorkloadKind::ConfigMap => {
            let api: Api<ConfigMap> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::configmap_summary)
                .collect()
        }
        WorkloadKind::Secret => {
            let api: Api<Secret> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::secret_summary)
                .collect()
        }
        WorkloadKind::NetworkPolicy => {
            let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::networkpolicy_summary)
                .collect()
        }
        WorkloadKind::PersistentVolumeClaim => {
            let api: Api<PersistentVolumeClaim> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::pvc_summary)
                .collect()
        }
        // Cluster-scoped — namespace arg ignored, use Api::all.
        WorkloadKind::PersistentVolume => {
            let api: Api<PersistentVolume> = Api::all(client);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::pv_summary)
                .collect()
        }
        WorkloadKind::StorageClass => {
            let api: Api<StorageClass> = Api::all(client);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::storage_class_summary)
                .collect()
        }
        WorkloadKind::IngressClass => {
            let api: Api<IngressClass> = Api::all(client);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::ingress_class_summary)
                .collect()
        }
        WorkloadKind::ResourceQuota => {
            let api: Api<ResourceQuota> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::resource_quota_summary)
                .collect()
        }
        WorkloadKind::HorizontalPodAutoscaler => {
            let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::hpa_summary)
                .collect()
        }
        // PR D+1 long-tail kinds.
        WorkloadKind::LimitRange => {
            let api: Api<LimitRange> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::limit_range_summary)
                .collect()
        }
        WorkloadKind::PodDisruptionBudget => {
            let api: Api<PodDisruptionBudget> = Api::namespaced(client, &namespace);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::pdb_summary)
                .collect()
        }
        WorkloadKind::PriorityClass => {
            let api: Api<PriorityClass> = Api::all(client);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::priority_class_summary)
                .collect()
        }
        WorkloadKind::MutatingWebhookConfiguration => {
            let api: Api<MutatingWebhookConfiguration> = Api::all(client);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::mutating_webhook_summary)
                .collect()
        }
        WorkloadKind::ValidatingWebhookConfiguration => {
            let api: Api<ValidatingWebhookConfiguration> = Api::all(client);
            api.list(&lp)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?
                .items
                .iter()
                .map(resources::validating_webhook_summary)
                .collect()
        }
        registered => list_dynamic_resources(client, &namespace, &registered).await?,
    };
    Ok(summaries)
}

#[tauri::command]
pub async fn get_resource(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ResourceDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    match kind {
        WorkloadKind::Deployment => {
            let api: Api<Deployment> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::deployment_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::StatefulSet => {
            let api: Api<StatefulSet> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::statefulset_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::DaemonSet => {
            let api: Api<DaemonSet> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::daemonset_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::CronJob => {
            let api: Api<CronJob> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::cronjob_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::Job => {
            let api: Api<Job> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::job_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::Pod => {
            let api: Api<Pod> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::pod_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::Service => {
            let api: Api<Service> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::service_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::Ingress => {
            let api: Api<Ingress> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::ingress_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::ConfigMap => {
            let api: Api<ConfigMap> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::configmap_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::Secret => {
            let api: Api<Secret> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::secret_summary(&obj);
            let raw_yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let yaml = resources::redact_secret_yaml(&raw_yaml)
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::NetworkPolicy => {
            let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::networkpolicy_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::PersistentVolumeClaim => {
            let api: Api<PersistentVolumeClaim> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::pvc_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::PersistentVolume => {
            let api: Api<PersistentVolume> = Api::all(client);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::pv_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::StorageClass => {
            let api: Api<StorageClass> = Api::all(client);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::storage_class_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::IngressClass => {
            let api: Api<IngressClass> = Api::all(client);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::ingress_class_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::ResourceQuota => {
            let api: Api<ResourceQuota> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::resource_quota_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::HorizontalPodAutoscaler => {
            let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::hpa_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::LimitRange => {
            let api: Api<LimitRange> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::limit_range_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::PodDisruptionBudget => {
            let api: Api<PodDisruptionBudget> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::pdb_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::PriorityClass => {
            let api: Api<PriorityClass> = Api::all(client);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::priority_class_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::MutatingWebhookConfiguration => {
            let api: Api<MutatingWebhookConfiguration> = Api::all(client);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::mutating_webhook_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        WorkloadKind::ValidatingWebhookConfiguration => {
            let api: Api<ValidatingWebhookConfiguration> = Api::all(client);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let summary = resources::validating_webhook_summary(&obj);
            let yaml =
                serde_yaml::to_string(&obj).map_err(|e| AppError::Internal(e.to_string()))?;
            let owner_refs = owner_refs_from(&obj.metadata);
            Ok(ResourceDetail {
                summary,
                yaml,
                owner_refs,
            })
        }
        registered => get_dynamic_resource(client, &namespace, &registered, &name).await,
    }
}

#[tauri::command]
pub async fn get_rbac_details(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<RbacDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    match kind {
        WorkloadKind::Role => {
            let api: Api<Role> = Api::namespaced(client, &namespace);
            let role = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(RbacDetail {
                role_ref: None,
                subjects: vec![],
                rules: rbac_details::rule_details(role.rules.as_ref()),
            })
        }
        WorkloadKind::ClusterRole => {
            let api: Api<ClusterRole> = Api::all(client);
            let role = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(RbacDetail {
                role_ref: None,
                subjects: vec![],
                rules: rbac_details::rule_details(role.rules.as_ref()),
            })
        }
        WorkloadKind::RoleBinding => {
            let api: Api<RoleBinding> = Api::namespaced(client.clone(), &namespace);
            let binding = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let rules = match binding.role_ref.kind.as_str() {
                "Role" => {
                    let role_api: Api<Role> = Api::namespaced(client, &namespace);
                    role_api
                        .get(&binding.role_ref.name)
                        .await
                        .ok()
                        .map(|role| rbac_details::rule_details(role.rules.as_ref()))
                        .unwrap_or_default()
                }
                "ClusterRole" => {
                    let role_api: Api<ClusterRole> = Api::all(client);
                    role_api
                        .get(&binding.role_ref.name)
                        .await
                        .ok()
                        .map(|role| rbac_details::rule_details(role.rules.as_ref()))
                        .unwrap_or_default()
                }
                _ => vec![],
            };
            Ok(RbacDetail {
                role_ref: Some(rbac_details::role_ref_label(&binding.role_ref)),
                subjects: rbac_details::subject_details(binding.subjects.as_ref()),
                rules,
            })
        }
        WorkloadKind::ClusterRoleBinding => {
            let api: Api<ClusterRoleBinding> = Api::all(client.clone());
            let binding = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let rules = if binding.role_ref.kind == "ClusterRole" {
                let role_api: Api<ClusterRole> = Api::all(client);
                role_api
                    .get(&binding.role_ref.name)
                    .await
                    .ok()
                    .map(|role| rbac_details::rule_details(role.rules.as_ref()))
                    .unwrap_or_default()
            } else {
                vec![]
            };
            Ok(RbacDetail {
                role_ref: Some(rbac_details::role_ref_label(&binding.role_ref)),
                subjects: rbac_details::subject_details(binding.subjects.as_ref()),
                rules,
            })
        }
        other => Err(AppError::Internal(format!(
            "RBAC details are not available for {other:?}"
        ))),
    }
}

#[tauri::command]
pub async fn get_storage_details(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<StorageDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    match kind {
        WorkloadKind::PersistentVolumeClaim => {
            let api: Api<PersistentVolumeClaim> = Api::namespaced(client, &namespace);
            let pvc = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(storage_details::pvc_detail(&pvc))
        }
        WorkloadKind::PersistentVolume => {
            let api: Api<PersistentVolume> = Api::all(client);
            let pv = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(storage_details::pv_detail(&pv))
        }
        WorkloadKind::StorageClass => {
            let api: Api<StorageClass> = Api::all(client);
            let class = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(storage_details::storage_class_detail(&class))
        }
        other => Err(AppError::Internal(format!(
            "storage details are not available for {other:?}"
        ))),
    }
}

#[tauri::command]
pub async fn get_resource_insights(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ResourceInsights> {
    let client = client_for(&state, context.as_deref()).await?;
    match kind {
        WorkloadKind::Service => {
            let api: Api<Service> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::service_insights(&obj))
        }
        WorkloadKind::Ingress => {
            let api: Api<Ingress> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::ingress_insights(&obj))
        }
        WorkloadKind::NetworkPolicy => {
            let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::network_policy_insights(&obj))
        }
        WorkloadKind::HorizontalPodAutoscaler => {
            let api: Api<HorizontalPodAutoscaler> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::hpa_insights(&obj))
        }
        WorkloadKind::PodDisruptionBudget => {
            let api: Api<PodDisruptionBudget> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::pdb_insights(&obj))
        }
        WorkloadKind::ResourceQuota => {
            let api: Api<ResourceQuota> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::quota_insights(&obj))
        }
        WorkloadKind::LimitRange => {
            let api: Api<LimitRange> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(resource_insights::limit_range_insights(&obj))
        }
        other => Err(AppError::Internal(format!(
            "resource insights are not available for {other:?}"
        ))),
    }
}

// ─── Streams ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn stream_events(
    namespace: Option<String>,
    stream_id: String,
    channel: tauri::ipc::Channel<crate::k8s::events::EventLine>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let client = client_for(&state, context.as_deref()).await?;
    let cancel = tokio_util::sync::CancellationToken::new();
    state
        .k8s
        .streams
        .write()
        .await
        .insert(stream_id.clone(), cancel.clone());
    tokio::spawn(async move {
        let _ = crate::k8s::events::stream_events(client, namespace, channel, cancel).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn watch_nodes(
    stream_id: String,
    channel: tauri::ipc::Channel<crate::k8s::watch::WatchEvent<crate::k8s::types::NodeSummary>>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let client = client_for(&state, context.as_deref()).await?;
    let cancel = tokio_util::sync::CancellationToken::new();
    state
        .k8s
        .streams
        .write()
        .await
        .insert(stream_id.clone(), cancel.clone());
    tokio::spawn(async move {
        let _ = crate::k8s::watch::watch_nodes(client, channel, cancel).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn watch_workloads(
    namespace: Option<String>,
    stream_id: String,
    channel: tauri::ipc::Channel<crate::k8s::watch::WatchEvent<crate::k8s::types::WorkloadSummary>>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let client = client_for(&state, context.as_deref()).await?;
    let cancel = tokio_util::sync::CancellationToken::new();
    state
        .k8s
        .streams
        .write()
        .await
        .insert(stream_id.clone(), cancel.clone());
    tokio::spawn(async move {
        let _ = crate::k8s::watch::watch_workloads(client, namespace, channel, cancel).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn stop_stream(stream_id: String, state: State<'_, AppState>) -> AppResult<()> {
    if let Some(c) = state.k8s.streams.write().await.remove(&stream_id) {
        c.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn stream_logs(
    selector: crate::k8s::logs::LogSelector,
    stream_id: String,
    channel: tauri::ipc::Channel<crate::k8s::logs::LogEvent>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let registry = state.k8s.clone();
    {
        let mut streams = registry.streams.write().await;
        if streams.contains_key(&stream_id) {
            return Err(AppError::K8s("log stream ID is already active".into()));
        }
        streams.insert(stream_id.clone(), cancel.clone());
    }
    // Register before client creation so stop_stream can cancel a slow startup.
    let client = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        result = client_for(&state, context.as_deref()) => result,
    };
    let client = match client {
        Ok(client) => client,
        Err(error) => {
            let mut streams = registry.streams.write().await;
            if !cancel.is_cancelled() {
                streams.remove(&stream_id);
            }
            return Err(error);
        }
    };
    tokio::spawn(async move {
        let result =
            crate::k8s::logs::stream_logs(client, selector, channel.clone(), cancel.clone()).await;
        if !cancel.is_cancelled() {
            match result {
                Ok(()) => crate::k8s::logs::emit_status(&channel, "", "ended", None),
                Err(error) => {
                    crate::k8s::logs::emit_status(&channel, "", "error", Some(error.to_string()))
                }
            }
        }
        let mut streams = registry.streams.write().await;
        // A cancelled entry was already removed by stop_stream. It may now
        // belong to a newer request with the same ID; leave that entry alone.
        if !cancel.is_cancelled() {
            streams.remove(&stream_id);
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn capture_incident_logs(
    selector: crate::k8s::logs::LogSelector,
    context: String,
    expected_uid: String,
    state: State<'_, AppState>,
) -> AppResult<crate::k8s::logs::BoundedLogCapture> {
    crate::k8s::logs::capture_logs_with_deadline(
        client_for(&state, Some(&context)),
        selector,
        20_000,
        &expected_uid,
        std::time::Duration::from_secs(5),
    )
    .await
}

fn label_selector_from(labels: &std::collections::BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[tauri::command]
pub async fn list_pods_for(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<WorkloadSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    match kind {
        WorkloadKind::Deployment => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let match_labels = obj
                .spec
                .as_ref()
                .and_then(|s| s.selector.match_labels.clone())
                .unwrap_or_default();
            if match_labels.is_empty() {
                return Ok(vec![]);
            }
            let ls = label_selector_from(
                &match_labels
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>(),
            );
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let list = pods
                .list(&ListParams::default().labels(&ls))
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(list.items.iter().map(resources::pod_summary).collect())
        }
        WorkloadKind::StatefulSet => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let match_labels = obj
                .spec
                .as_ref()
                .and_then(|s| s.selector.match_labels.clone())
                .unwrap_or_default();
            if match_labels.is_empty() {
                return Ok(vec![]);
            }
            let ls = label_selector_from(
                &match_labels
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>(),
            );
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let list = pods
                .list(&ListParams::default().labels(&ls))
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(list.items.iter().map(resources::pod_summary).collect())
        }
        WorkloadKind::DaemonSet => {
            let api: Api<DaemonSet> = Api::namespaced(client.clone(), &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let match_labels = obj
                .spec
                .as_ref()
                .and_then(|s| s.selector.match_labels.clone())
                .unwrap_or_default();
            if match_labels.is_empty() {
                return Ok(vec![]);
            }
            let ls = label_selector_from(
                &match_labels
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>(),
            );
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let list = pods
                .list(&ListParams::default().labels(&ls))
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(list.items.iter().map(resources::pod_summary).collect())
        }
        WorkloadKind::CronJob => {
            let jobs_api: Api<Job> = Api::namespaced(client.clone(), &namespace);
            let jobs_list = jobs_api
                .list(&ListParams::default())
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let owned_jobs: Vec<String> = jobs_list
                .items
                .iter()
                .filter(|j| {
                    j.metadata
                        .owner_references
                        .as_ref()
                        .map(|v| v.iter().any(|o| o.kind == "CronJob" && o.name == name))
                        .unwrap_or(false)
                })
                .filter_map(|j| j.metadata.name.clone())
                .collect();
            if owned_jobs.is_empty() {
                return Ok(vec![]);
            }
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let pod_list = pods
                .list(&ListParams::default())
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let filtered: Vec<WorkloadSummary> = pod_list
                .items
                .iter()
                .filter(|p| {
                    p.metadata
                        .owner_references
                        .as_ref()
                        .map(|v| {
                            v.iter()
                                .any(|o| o.kind == "Job" && owned_jobs.contains(&o.name))
                        })
                        .unwrap_or(false)
                })
                .map(resources::pod_summary)
                .collect();
            Ok(filtered)
        }
        WorkloadKind::Job => {
            let pods: Api<Pod> = Api::namespaced(client, &namespace);
            let pod_list = pods
                .list(&ListParams::default())
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            let filtered: Vec<WorkloadSummary> = pod_list
                .items
                .iter()
                .filter(|p| {
                    p.metadata
                        .owner_references
                        .as_ref()
                        .map(|v| v.iter().any(|o| o.kind == "Job" && o.name == name))
                        .unwrap_or(false)
                })
                .map(resources::pod_summary)
                .collect();
            Ok(filtered)
        }
        WorkloadKind::Pod => {
            let api: Api<Pod> = Api::namespaced(client, &namespace);
            let obj = api
                .get(&name)
                .await
                .map_err(|e| AppError::K8s(e.to_string()))?;
            Ok(vec![resources::pod_summary(&obj)])
        }
        _ => Ok(vec![]),
    }
}

/// List every pod scheduled on a single node. Uses a server-side
/// `spec.nodeName=<node>` field selector so we don't drag the whole
/// pod set across the wire on large clusters, then folds in metrics-server
/// usage when available (same enrichment as the workloads pod table).
#[tauri::command]
pub async fn list_pods_on_node(
    node: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<WorkloadSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    let api: Api<Pod> = Api::all(client.clone());
    let lp = ListParams::default().fields(&format!("spec.nodeName={node}"));
    let mut summaries: Vec<WorkloadSummary> = api
        .list(&lp)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?
        .items
        .iter()
        .map(resources::pod_summary)
        .collect();
    let ctx_name = context.as_deref().unwrap_or("");
    if !ctx_name.is_empty() {
        if let Some(usage) = crate::k8s::metrics::pod_usage(&client, ctx_name).await {
            let mut by_key: std::collections::HashMap<(String, String), (i64, i64)> =
                std::collections::HashMap::with_capacity(usage.len());
            for u in usage {
                by_key.insert((u.namespace, u.name), (u.cpu_milli, u.mem_bytes));
            }
            for s in summaries.iter_mut() {
                if let Some((cpu, mem)) = by_key.get(&(s.namespace.clone(), s.name.clone())) {
                    s.cpu_milli = Some(*cpu);
                    s.mem_bytes = Some(*mem);
                }
            }
        }
    }
    Ok(summaries)
}

// ─── CRD Browser ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_crds(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<crd_mod::CrdSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    crd_mod::list_crds(&client).await
}

#[tauri::command]
pub async fn list_cr_instances(
    group: String,
    version: String,
    kind: String,
    plural: String,
    namespace: Option<String>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<crd_mod::CrInstance>> {
    let client = client_for(&state, context.as_deref()).await?;
    crd_mod::list_instances(&client, &group, &version, &kind, &plural, namespace).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_cr_yaml(
    group: String,
    version: String,
    kind: String,
    plural: String,
    namespace: Option<String>,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let client = client_for(&state, context.as_deref()).await?;
    crd_mod::get_instance_yaml(&client, &group, &version, &kind, &plural, namespace, &name).await
}

// ─── Team Access (RBAC provisioning) ──────────────────────────────────────

#[tauri::command]
pub async fn provision_team_access(
    request: rbac_admin::TeamAccessRequest,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<rbac_admin::TeamAccessResult> {
    let (ctx, client, _, kc) = mutation_client(&state, context.as_deref(), false).await?;
    rbac_admin::provision(&client, &ctx, &kc, request).await
}

#[tauri::command]
pub async fn revoke_team_access(
    member_id: String,
    namespaces: Vec<String>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<rbac_admin::CreatedObject>> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    rbac_admin::revoke(&client, &member_id, namespaces).await
}

#[tauri::command]
pub async fn list_team_access(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<rbac_admin::TeamGrant>> {
    let client = client_for(&state, context.as_deref()).await?;
    rbac_admin::list_grants(&client).await
}

#[tauri::command]
pub async fn renew_team_token(
    member_id: String,
    ttl_hours: i64,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<rbac_admin::TeamAccessResult> {
    let (ctx, client, _, kc) = mutation_client(&state, context.as_deref(), false).await?;
    rbac_admin::renew_token(&client, &ctx, &kc, &member_id, ttl_hours).await
}

#[tauri::command]
pub async fn rotate_team_token(
    member_id: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<rbac_admin::TeamAccessResult> {
    let (ctx, client, _, kc) = mutation_client(&state, context.as_deref(), false).await?;
    rbac_admin::rotate_long_lived_token(&client, &ctx, &kc, &member_id).await
}

// ─── Workload actions (writes) + events ───────────────────────────────────

#[tauri::command]
pub async fn list_events_for(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<act::EventSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    act::list_events_for(&client, &namespace, kind, &name).await
}

#[tauri::command]
pub async fn restart_workload(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::rollout_restart(&client, &namespace, kind, &name).await
}

#[tauri::command]
pub async fn scale_workload(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    replicas: i32,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::scale(&client, &namespace, kind, &name, replicas).await
}

#[tauri::command]
pub async fn detect_trivy() -> bool {
    crate::k8s::vulnscan::is_trivy_available().await
}

#[tauri::command]
pub async fn scan_image(image: String) -> AppResult<crate::k8s::vulnscan::VulnReport> {
    crate::k8s::vulnscan::scan_image(&image).await
}

#[tauri::command]
pub async fn list_manual_cronjob_runs(
    namespace: String,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<act::ManualRunSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    act::list_manual_cronjob_runs(&client, &namespace, &name).await
}

#[tauri::command]
pub async fn set_workload_image(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    container: String,
    image: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::set_workload_image(&client, &namespace, kind, &name, &container, &image).await
}

#[tauri::command]
pub async fn delete_pod(
    namespace: String,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::delete_pod(&client, &namespace, &name).await
}

/// Trigger a manual run of a CronJob. Returns the name of the freshly-created Job.
#[tauri::command]
pub async fn trigger_cronjob(
    namespace: String,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::trigger_cronjob(&client, &namespace, &name).await
}

#[tauri::command]
pub async fn cordon_node(
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::cordon_node(&client, &name).await
}

#[tauri::command]
pub async fn uncordon_node(
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::uncordon_node(&client, &name).await
}

#[tauri::command]
pub async fn drain_node(
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<act::DrainSummary> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::drain_node(&client, &name).await
}

#[tauri::command]
pub async fn delete_resource(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    act::delete_resource(&client, &namespace, kind, &name).await
}

// ─── Port-forward ─────────────────────────────────────────────────────────

use crate::k8s::portforward;

#[tauri::command]
pub async fn start_port_forward(
    namespace: String,
    target_kind: portforward::ForwardTargetKind,
    target_name: String,
    local_port: u16,
    remote_port: u16,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<portforward::ForwardSession> {
    let ctx = state.k8s.resolve_context(context.as_deref()).await?;
    let client = state.k8s.client_for(&ctx).await?;
    portforward::start(
        client,
        state.forwards.clone(),
        portforward::StartForwardRequest {
            context: ctx,
            namespace,
            target_kind,
            target_name,
            local_port,
            remote_port,
        },
    )
    .await
}

#[tauri::command]
pub async fn list_port_forwards(
    state: State<'_, AppState>,
) -> AppResult<Vec<portforward::ForwardSession>> {
    Ok(state.forwards.list().await)
}

#[tauri::command]
pub async fn stop_port_forward(id: String, state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.forwards.stop(&id).await)
}

// ─── Pod container list (for log picker, future exec) ─────────────────────

#[derive(serde::Serialize)]
pub struct PodContainerInfo {
    pub name: String,
    pub image: String,
    pub is_init: bool,
    pub is_default: bool,
}

#[tauri::command]
pub async fn list_pod_containers(
    namespace: String,
    pod: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<PodContainerInfo>> {
    let client = client_for(&state, context.as_deref()).await?;
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let obj = api
        .get(&pod)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;
    let default_name = obj
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get("kubectl.kubernetes.io/default-container"))
        .cloned();
    let spec = obj
        .spec
        .ok_or_else(|| AppError::K8s("pod has no spec".into()))?;
    const SIDECAR: &[&str] = &[
        "istio-proxy",
        "istio-init",
        "linkerd-proxy",
        "linkerd-init",
        "envoy",
        "secret-agent",
        "cloud-sql-proxy",
        "otel-agent",
    ];
    let fallback_default: Option<String> = default_name.clone().or_else(|| {
        spec.containers
            .iter()
            .find(|c| !SIDECAR.iter().any(|s| c.name.contains(s)))
            .map(|c| c.name.clone())
            .or_else(|| spec.containers.first().map(|c| c.name.clone()))
    });
    let mut out: Vec<PodContainerInfo> = Vec::new();
    for c in &spec.containers {
        out.push(PodContainerInfo {
            name: c.name.clone(),
            image: c.image.clone().unwrap_or_default(),
            is_init: false,
            is_default: Some(c.name.clone()) == fallback_default,
        });
    }
    if let Some(inits) = &spec.init_containers {
        for c in inits {
            out.push(PodContainerInfo {
                name: c.name.clone(),
                image: c.image.clone().unwrap_or_default(),
                is_init: true,
                is_default: false,
            });
        }
    }
    for c in spec.ephemeral_containers.as_deref().unwrap_or_default() {
        out.push(PodContainerInfo {
            name: c.name.clone(),
            image: c.image.clone().unwrap_or_default(),
            is_init: false,
            is_default: false,
        });
    }
    Ok(out)
}

// ─── YAML apply ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn apply_resource(
    namespace: String,
    kind: WorkloadKind,
    name: String,
    yaml: String,
    dry_run: bool,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<act::ApplyOutcome> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), dry_run).await?;
    act::apply_resource(&client, &namespace, kind, &name, &yaml, dry_run).await
}

// ─── Helm browser (read-only) ─────────────────────────────────────────────

use crate::k8s::helm;

#[tauri::command]
pub async fn list_helm_releases(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<helm::HelmReleaseSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    helm::list_releases(&client).await
}

#[tauri::command]
pub async fn get_helm_release(
    namespace: String,
    name: String,
    revision: Option<i32>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<helm::HelmReleaseDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    helm::get_release(&client, &namespace, &name, revision).await
}

#[tauri::command]
pub async fn list_helm_history(
    namespace: String,
    name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<helm::HelmReleaseSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    helm::list_history(&client, &namespace, &name).await
}

// ─── Helm write ops via shell-out ─────────────────────────────────────────

use crate::k8s::helm_cli;

async fn register_helm_stream(
    state: &State<'_, AppState>,
    stream_id: &str,
) -> tokio_util::sync::CancellationToken {
    let cancel = tokio_util::sync::CancellationToken::new();
    state
        .k8s
        .streams
        .write()
        .await
        .insert(stream_id.to_string(), cancel.clone());
    cancel
}

#[tauri::command]
pub async fn helm_install(
    request: helm_cli::HelmInstallRequest,
    stream_id: String,
    channel: tauri::ipc::Channel<helm_cli::HelmEvent>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (ctx, config, identity) =
        mutation_target(&state, context.as_deref(), request.dry_run).await?;
    let snapshot = helm_cli::ConfigSnapshot::new(&config)?;
    state
        .protection
        .require_mutation(&ctx, &identity, request.dry_run)?;
    let cancel = register_helm_stream(&state, &stream_id).await;
    let policy = state.protection.clone();
    tokio::spawn(async move {
        if let Err(error) = require_current_target(&policy, &ctx, &identity, request.dry_run) {
            let _ = channel.send(helm_cli::HelmEvent::Error {
                message: error.to_string(),
            });
            return;
        }
        let _ = helm_cli::install(&ctx, snapshot, request, channel, cancel).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn helm_upgrade(
    request: helm_cli::HelmUpgradeRequest,
    stream_id: String,
    channel: tauri::ipc::Channel<helm_cli::HelmEvent>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (ctx, config, identity) =
        mutation_target(&state, context.as_deref(), request.dry_run).await?;
    let snapshot = helm_cli::ConfigSnapshot::new(&config)?;
    state
        .protection
        .require_mutation(&ctx, &identity, request.dry_run)?;
    let cancel = register_helm_stream(&state, &stream_id).await;
    let policy = state.protection.clone();
    tokio::spawn(async move {
        if let Err(error) = require_current_target(&policy, &ctx, &identity, request.dry_run) {
            let _ = channel.send(helm_cli::HelmEvent::Error {
                message: error.to_string(),
            });
            return;
        }
        let _ = helm_cli::upgrade(&ctx, snapshot, request, channel, cancel).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn helm_rollback(
    request: helm_cli::HelmRollbackRequest,
    stream_id: String,
    channel: tauri::ipc::Channel<helm_cli::HelmEvent>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (ctx, config, identity) =
        mutation_target(&state, context.as_deref(), request.dry_run).await?;
    let snapshot = helm_cli::ConfigSnapshot::new(&config)?;
    state
        .protection
        .require_mutation(&ctx, &identity, request.dry_run)?;
    let cancel = register_helm_stream(&state, &stream_id).await;
    let policy = state.protection.clone();
    tokio::spawn(async move {
        if let Err(error) = require_current_target(&policy, &ctx, &identity, request.dry_run) {
            let _ = channel.send(helm_cli::HelmEvent::Error {
                message: error.to_string(),
            });
            return;
        }
        let _ = helm_cli::rollback(&ctx, snapshot, request, channel, cancel).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn helm_uninstall(
    request: helm_cli::HelmUninstallRequest,
    stream_id: String,
    channel: tauri::ipc::Channel<helm_cli::HelmEvent>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (ctx, config, identity) = mutation_target(&state, context.as_deref(), false).await?;
    let snapshot = helm_cli::ConfigSnapshot::new(&config)?;
    state.protection.require_mutation(&ctx, &identity, false)?;
    let cancel = register_helm_stream(&state, &stream_id).await;
    let policy = state.protection.clone();
    tokio::spawn(async move {
        if let Err(error) = require_current_target(&policy, &ctx, &identity, false) {
            let _ = channel.send(helm_cli::HelmEvent::Error {
                message: error.to_string(),
            });
            return;
        }
        let _ = helm_cli::uninstall(&ctx, snapshot, request, channel, cancel).await;
    });
    Ok(())
}

/// Search configured helm repos for charts matching `query`. Empty repos
/// or no matches return an empty list (not an error) so the install wizard's
/// manual-entry path keeps working when the user has no repos added yet.
#[tauri::command]
pub async fn helm_search_repo(query: String) -> AppResult<Vec<helm_cli::ChartHit>> {
    helm_cli::search_repo(&query).await
}

/// Returns the chart's default values.yaml as a string. Used by the install
/// wizard to seed the values editor.
#[tauri::command]
pub async fn helm_show_values(chart: String, version: Option<String>) -> AppResult<String> {
    helm_cli::show_values(&chart, version.as_deref()).await
}

// ─── Pod attach (terminal) ────────────────────────────────────────────────

use crate::k8s::exec as attach;

#[tauri::command]
pub async fn get_debug_target(
    context: String,
    namespace: String,
    pod: String,
    state: State<'_, AppState>,
) -> AppResult<crate::k8s::debug::DebugTarget> {
    let client = client_for(&state, Some(&context)).await?;
    crate::k8s::debug::get_target(client, &context, &namespace, &pod).await
}

#[tauri::command]
pub async fn create_debug_container(
    request: crate::k8s::debug::DebugRequest,
    state: State<'_, AppState>,
) -> AppResult<crate::k8s::debug::DebugResult> {
    let (context, client, identity, _) =
        mutation_client(&state, Some(&request.context), false).await?;
    crate::k8s::debug::create(client, request, || {
        require_current_target(&state.protection, &context, &identity, false)
    })
    .await
}

#[tauri::command]
pub async fn start_pod_attach(
    request: attach::StartAttachRequest,
    channel: tauri::ipc::Channel<attach::AttachEvent>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let (ctx, client, identity, _) = mutation_client(&state, context.as_deref(), false).await?;
    attach::start(
        client,
        state.attachments.clone(),
        request,
        channel,
        attach::SessionProtection {
            context: ctx,
            identity,
            policy: state.protection.clone(),
        },
    )
    .await
}

#[tauri::command]
pub async fn pod_attach_stdin(
    id: String,
    bytes: Vec<u8>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.attachments.write_stdin(&id, bytes).await
}

#[tauri::command]
pub async fn pod_attach_resize(
    id: String,
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state.attachments.resize(&id, cols, rows).await
}

#[tauri::command]
pub async fn pod_attach_close(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.attachments.close(&id).await;
    Ok(())
}

// ─── Pod details (Lens-style detail drawer) ───────────────────────────────

/// Fetch the rich detail payload for a single pod: metadata, conditions,
/// containers (with resource requests/limits), live cpu/memory from
/// metrics-server cache, and IP/node/QoS metadata. Used by the
/// `ResourceDetailDrawer` Properties tab.
#[tauri::command]
pub async fn get_pod_details(
    ctx: String,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<PodDetails> {
    let client = client_for(&state, Some(&ctx)).await?;
    let api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let pod = api
        .get(&name)
        .await
        .map_err(|e| AppError::K8s(e.to_string()))?;

    let meta = &pod.metadata;
    let spec = pod.spec.as_ref();
    let status = pod.status.as_ref();

    let phase = status.and_then(|s| s.phase.clone()).unwrap_or_default();
    let qos_class = status
        .and_then(|s| s.qos_class.clone())
        .unwrap_or_else(|| "BestEffort".to_string());
    let node_name = spec.and_then(|s| s.node_name.clone());
    let pod_ip = status.and_then(|s| s.pod_ip.clone());
    let pod_ips: Vec<String> = status
        .and_then(|s| s.pod_ips.as_ref())
        .map(|v| v.iter().map(|p| p.ip.clone()).collect())
        .unwrap_or_default();
    let service_account = spec.and_then(|s| s.service_account_name.clone());
    let tolerations = spec
        .and_then(|s| s.tolerations.as_ref())
        .map(|t| t.len() as i32)
        .unwrap_or(0);
    let conditions: Vec<PodCondition> = status
        .and_then(|s| s.conditions.as_ref())
        .map(|cs| {
            cs.iter()
                .map(|c| PodCondition {
                    r#type: c.type_.clone(),
                    status: c.status.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let controlled_by = meta
        .owner_references
        .as_ref()
        .and_then(|v| v.first())
        .map(|o| OwnerRefLite {
            kind: o.kind.clone(),
            name: o.name.clone(),
        });

    let labels: std::collections::BTreeMap<String, String> = meta
        .labels
        .clone()
        .map(|m| m.into_iter().collect())
        .unwrap_or_default();
    let annotations: std::collections::BTreeMap<String, String> = meta
        .annotations
        .clone()
        .map(|m| m.into_iter().collect())
        .unwrap_or_default();

    // Build a container_status lookup so we can fold per-container restart
    // count + state into the spec-driven container list.
    let cs_by_name: std::collections::HashMap<String, _> = status
        .and_then(|s| s.container_statuses.as_ref())
        .map(|v| v.iter().map(|cs| (cs.name.clone(), cs.clone())).collect())
        .unwrap_or_default();

    let containers: Vec<ContainerInfo> = spec
        .map(|s| {
            s.containers
                .iter()
                .map(|c| {
                    let cs = cs_by_name.get(&c.name);
                    let ready = cs.map(|s| s.ready).unwrap_or(false);
                    let restart_count = cs.map(|s| s.restart_count).unwrap_or(0);
                    let state = cs
                        .and_then(|s| s.state.as_ref())
                        .map(|st| {
                            if st.running.is_some() {
                                "Running".to_string()
                            } else if let Some(w) = &st.waiting {
                                format!(
                                    "Waiting:{}",
                                    w.reason.clone().unwrap_or_else(|| "Unknown".into())
                                )
                            } else if let Some(t) = &st.terminated {
                                format!(
                                    "Terminated:{}",
                                    t.reason.clone().unwrap_or_else(|| "Unknown".into())
                                )
                            } else {
                                "Unknown".into()
                            }
                        })
                        .unwrap_or_else(|| "Unknown".into());
                    let (cpu_req, cpu_lim, mem_req, mem_lim) = c
                        .resources
                        .as_ref()
                        .map(|r| {
                            let req = r.requests.as_ref();
                            let lim = r.limits.as_ref();
                            (
                                req.and_then(|m| m.get("cpu"))
                                    .and_then(|q| metrics::parse_cpu_milli(&q.0)),
                                lim.and_then(|m| m.get("cpu"))
                                    .and_then(|q| metrics::parse_cpu_milli(&q.0)),
                                req.and_then(|m| m.get("memory"))
                                    .and_then(|q| metrics::parse_memory_bytes(&q.0)),
                                lim.and_then(|m| m.get("memory"))
                                    .and_then(|q| metrics::parse_memory_bytes(&q.0)),
                            )
                        })
                        .unwrap_or((None, None, None, None));
                    ContainerInfo {
                        name: c.name.clone(),
                        image: c.image.clone().unwrap_or_default(),
                        ready,
                        restart_count,
                        state,
                        cpu_request_milli: cpu_req,
                        cpu_limit_milli: cpu_lim,
                        mem_request_bytes: mem_req,
                        mem_limit_bytes: mem_lim,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Pull live usage from the metrics-server cache (10s TTL, free).
    let (cpu_usage_milli, mem_usage_bytes) = match metrics::pod_usage(&client, &ctx).await {
        Some(usage) => usage
            .into_iter()
            .find(|u| u.name == name && u.namespace == namespace)
            .map(|u| (Some(u.cpu_milli), Some(u.mem_bytes)))
            .unwrap_or((None, None)),
        None => (None, None),
    };

    let created_at_ms = meta
        .creation_timestamp
        .as_ref()
        .map(time::millis)
        .unwrap_or(0);
    let age_seconds = if created_at_ms > 0 {
        ((chrono::Utc::now().timestamp_millis() - created_at_ms) / 1000).max(0)
    } else {
        0
    };

    Ok(PodDetails {
        name,
        namespace,
        status: phase,
        qos_class,
        node_name,
        controlled_by,
        service_account,
        pod_ip,
        pod_ips,
        conditions,
        tolerations,
        labels,
        annotations,
        containers,
        cpu_usage_milli,
        mem_usage_bytes,
        age_seconds,
        created_at_ms,
    })
}

// ─── ArgoCD ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn detect_argocd(context: Option<String>, state: State<'_, AppState>) -> AppResult<bool> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::detect(&client).await
}

#[tauri::command]
pub async fn list_argocd_applications(
    context: Option<String>,
    namespace: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<argocd::ApplicationSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::list_applications(&client, namespace.as_deref()).await
}

#[tauri::command]
pub async fn get_argocd_application(
    context: Option<String>,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<argocd::ApplicationDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::get_application(&client, &namespace, &name).await
}

#[tauri::command]
pub async fn sync_argocd_application(
    context: Option<String>,
    namespace: String,
    name: String,
    options: argocd::SyncOptions,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    argocd::sync_application(&client, &namespace, &name, &options).await
}

#[tauri::command]
pub async fn refresh_argocd_application(
    context: Option<String>,
    namespace: String,
    name: String,
    hard: bool,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    argocd::refresh_application(&client, &namespace, &name, hard).await
}

#[tauri::command]
pub async fn terminate_argocd_operation(
    context: Option<String>,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    argocd::terminate_operation(&client, &namespace, &name).await
}

#[tauri::command]
pub async fn detect_argocd_application_sets(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::detect_application_sets(&client).await
}

#[tauri::command]
pub async fn list_argocd_application_sets(
    context: Option<String>,
    namespace: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<argocd::ApplicationSetSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::list_application_sets(&client, namespace.as_deref()).await
}

#[tauri::command]
pub async fn get_argocd_application_set(
    context: Option<String>,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<argocd::ApplicationSetDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::get_application_set(&client, &namespace, &name).await
}

#[tauri::command]
pub async fn list_argocd_app_projects(
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<argocd::AppProjectSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::list_app_projects(&client).await
}

#[tauri::command]
pub async fn get_argocd_app_project(
    context: Option<String>,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<argocd::AppProjectDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    argocd::get_app_project(&client, &namespace, &name).await
}

// ─── Tekton ───────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn detect_tekton(context: Option<String>, state: State<'_, AppState>) -> AppResult<bool> {
    let client = client_for(&state, context.as_deref()).await?;
    tekton::detect(&client).await
}

#[tauri::command]
pub async fn list_pipeline_runs(
    context: Option<String>,
    namespace: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<tekton::PipelineRunSummary>> {
    let client = client_for(&state, context.as_deref()).await?;
    tekton::list_pipeline_runs(&client, namespace.as_deref()).await
}

#[tauri::command]
pub async fn get_pipeline_run(
    context: Option<String>,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<tekton::PipelineRunDetail> {
    let client = client_for(&state, context.as_deref()).await?;
    tekton::get_pipeline_run(&client, &namespace, &name).await
}

#[tauri::command]
pub async fn cancel_pipeline_run(
    context: Option<String>,
    namespace: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (_, client, _, _) = mutation_client(&state, context.as_deref(), false).await?;
    tekton::cancel_pipeline_run(&client, &namespace, &name).await
}

#[cfg(test)]
mod protection_entrypoint_tests {
    /// Every cluster write IPC belongs here; adding an IPC requires classifying it.
    const MUTATIONS: &[&str] = &[
        "provision_team_access",
        "revoke_team_access",
        "renew_team_token",
        "rotate_team_token",
        "restart_workload",
        "scale_workload",
        "set_workload_image",
        "delete_pod",
        "trigger_cronjob",
        "cordon_node",
        "uncordon_node",
        "drain_node",
        "delete_resource",
        "apply_resource",
        "helm_install",
        "helm_upgrade",
        "helm_rollback",
        "helm_uninstall",
        "start_pod_attach",
        "create_debug_container",
        "sync_argocd_application",
        "refresh_argocd_application",
        "terminate_argocd_operation",
        "cancel_pipeline_run",
    ];
    #[test]
    fn all_cluster_mutation_entrypoints_require_native_authorization() {
        let source = include_str!("k8s.rs");
        for name in MUTATIONS {
            let marker = format!("pub async fn {name}(");
            let body = source
                .split_once(&marker)
                .unwrap()
                .1
                .split("\n}")
                .next()
                .unwrap();
            assert!(
                body.contains("mutation_client(") || body.contains("mutation_target("),
                "{name} has no native mutation guard"
            );
        }
    }

    #[test]
    fn every_registered_kubernetes_ipc_has_an_explicit_policy_classification() {
        let read_or_local = &[
            "list_contexts",
            "get_debug_target",
            "set_context",
            "delete_context",
            "list_deleted_contexts",
            "restore_deleted_context",
            "list_namespaces",
            "list_fleet",
            "probe_fleet_context",
            "disconnect_context",
            "reconnect_all",
            "list_nodes",
            "metrics_explorer_snapshot",
            "cloud_map",
            "network_debug_snapshot",
            "security_scan",
            "check_access",
            "list_workloads",
            "get_resource",
            "get_rbac_details",
            "get_storage_details",
            "get_resource_insights",
            "stream_events",
            "watch_nodes",
            "watch_workloads",
            "stop_stream",
            "stream_logs",
            "capture_incident_logs",
            "list_pods_for",
            "list_pods_on_node",
            "list_crds",
            "list_cr_instances",
            "get_cr_yaml",
            "list_team_access",
            "list_events_for",
            "detect_trivy",
            "scan_image",
            "list_manual_cronjob_runs",
            "start_port_forward",
            "list_port_forwards",
            "stop_port_forward",
            "list_pod_containers",
            "list_helm_releases",
            "get_helm_release",
            "list_helm_history",
            "helm_search_repo",
            "helm_show_values",
            "get_pod_details",
            "detect_argocd",
            "list_argocd_applications",
            "get_argocd_application",
            "detect_argocd_application_sets",
            "list_argocd_application_sets",
            "get_argocd_application_set",
            "list_argocd_app_projects",
            "get_argocd_app_project",
            "detect_tekton",
            "list_pipeline_runs",
            "get_pipeline_run",
        ];
        let policy_and_session_controls = &[
            "get_context_protection",
            "set_context_protection",
            "unlock_context",
            "lock_context",
            "pod_attach_stdin",
            "pod_attach_resize",
            "pod_attach_close",
        ];
        let classified: std::collections::BTreeSet<&str> = MUTATIONS
            .iter()
            .chain(read_or_local.iter())
            .chain(policy_and_session_controls.iter())
            .copied()
            .collect();
        let source = include_str!("../lib.rs");
        let registered: std::collections::BTreeSet<&str> = source
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("commands::k8s::")
                    .map(|name| name.trim_end_matches(','))
            })
            .collect();
        assert_eq!(
            registered, classified,
            "Every new IPC needs a mutation-policy audit"
        );
    }

    #[tokio::test]
    async fn native_guard_blocks_api_writes_and_allows_server_dry_run() {
        use wiremock::{
            matchers::{method, path, query_param},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let config: kube::config::Kubeconfig = serde_yaml::from_str(&format!("apiVersion: v1\nkind: Config\nclusters:\n- name: target\n  cluster:\n    server: {}\ncontexts:\n- name: prod\n  context:\n    cluster: target\ncurrent-context: prod\n", server.uri())).unwrap();
        let folder = std::env::temp_dir().join(format!(
            "lumen-native-guard-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let state = super::AppState::with_config_dir(folder.clone());
        let identity = crate::protection::identity_from_config(&config, "prod").unwrap();
        state
            .protection
            .set_protected("prod", true, &identity)
            .unwrap();
        assert!(matches!(
            super::mutation_client_from_config(
                &state,
                "prod".into(),
                config.clone(),
                identity.clone(),
                false
            )
            .await,
            Err(crate::error::AppError::PermissionDenied(_))
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
        Mock::given(method("PATCH")).and(path("/api/v1/namespaces/default/configmaps/example")).and(query_param("dryRun", "All"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"apiVersion":"v1", "kind":"ConfigMap", "metadata":{"name":"example", "namespace":"default"}, "data":{"hello":"world"}}))).expect(1).mount(&server).await;
        let (_, client, _, _) = super::mutation_client_from_config(
            &state,
            "prod".into(),
            config.clone(),
            identity.clone(),
            true,
        )
        .await
        .unwrap();
        let result = crate::k8s::actions::apply_resource(
            &client,
            "default",
            crate::k8s::types::WorkloadKind::ConfigMap,
            "example",
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: example\ndata:\n  hello: world\n",
            true,
        )
        .await
        .unwrap();
        assert!(result.dry_run);
        state.protection.unlock("prod", &identity).unwrap();
        assert!(super::mutation_client_from_config(
            &state,
            "prod".into(),
            config.clone(),
            identity.clone(),
            false
        )
        .await
        .is_ok());
        state.protection.lock("prod", &identity).unwrap();
        assert!(super::mutation_client_from_config(
            &state,
            "prod".into(),
            config,
            identity.clone(),
            false
        )
        .await
        .is_err());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[tokio::test]
    async fn mutation_cannot_use_legacy_current_context() {
        let state = super::AppState::new();
        *state.k8s.current_context.write().await = Some("prod".into());
        assert!(matches!(
            super::mutation_target(&state, None, false).await,
            Err(crate::error::AppError::PermissionDenied(_))
        ));
        assert!(super::mutation_target(&state, Some(" "), false)
            .await
            .is_err());
    }
    #[test]
    fn network_policy_adapter_preserves_egress_empty_selectors_ranges_and_unknown_expressions() {
        let policy: super::NetworkPolicy = serde_json::from_value(serde_json::json!({
            "metadata": { "name": "example", "namespace": "shop" },
            "spec": {
                "podSelector": { "matchExpressions": [{ "key": "app", "operator": "Exists" }] },
                "policyTypes": ["Ingress", "Egress"],
                "ingress": [],
                "egress": [{ "to": [{ "namespaceSelector": {}, "podSelector": { "matchExpressions": [{ "key": "app", "operator": "Exists" }] } }], "ports": [{ "port": 8000, "endPort": 9000 }] }]
            }
        })).unwrap();
        let output = super::network_policy_resource(&policy);
        assert!(output.unsupported);
        let rule = &output.egress.as_ref().unwrap()[0];
        assert!(rule.to[0].namespace_selector.as_ref().unwrap().is_empty());
        assert!(rule.to[0].unsupported);
        assert_eq!(rule.ports[0].end_port, Some(9000));
        let json = serde_json::to_value(output).unwrap();
        assert_eq!(json["egress"][0]["ports"][0]["endPort"], 9000);
    }
    #[tokio::test]
    async fn network_snapshot_preserves_partial_results_and_explicit_denials() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"apiVersion":"v1", "kind":"Status", "status":"Failure", "message":"forbidden", "reason":"Forbidden", "code":403}))).with_priority(10).mount(&server).await;
        Mock::given(method("GET")).and(path("/api/v1/namespaces/shop/pods")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"apiVersion":"v1", "kind":"PodList", "metadata":{}, "items":[{"metadata":{"name":"api", "namespace":"shop"}, "spec":{"containers":[]}}]}))).with_priority(1).mount(&server).await;
        let config = kube::Config::new(server.uri().parse().unwrap());
        let client = kube::Client::try_from(config).unwrap();
        let result = super::network_debug_snapshot_for(client, "shop".into())
            .await
            .unwrap();
        assert_eq!(result.pods.len(), 1);
        assert_eq!(result.pods[0].name, "api");
        assert!(result.network_policies.is_empty());
        assert!(result.unavailable.contains_key("shop/networkPolicies"));
        assert!(result.unavailable.contains_key("shop/gateways"));
        assert!(!result.unavailable.contains_key("shop/pods"));
    }
}
