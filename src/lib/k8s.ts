import type { DebugRequest, DebugResult, DebugTarget } from "./podDebug";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { assertContextMutation } from "@/lib/contextProtection";
import { useUiSettings } from "@/state/uiSettings";
import type { NetworkDebugSnapshot } from "./networkDebugger";
import type {
  ChangeHistoryAction,
  ChangeHistoryEventInput,
  ChangeHistoryTarget,
} from "@/lib/changeHistory";
import { useChangeHistoryStore } from "@/state/changeHistory";

// All cluster writes and interactive execution cross this single UI guard.
// Native code is the final authority and checks again before the side effect.
const mutationCommands = new Set([
  "provision_team_access", "revoke_team_access", "renew_team_token", "rotate_team_token",
  "restart_workload", "scale_workload", "set_workload_image", "delete_pod",
  "cordon_node", "uncordon_node", "drain_node", "trigger_cronjob", "delete_resource",
  "apply_resource", "helm_install", "helm_upgrade", "helm_rollback", "helm_uninstall",
  "create_debug_container", "start_pod_attach", "sync_argocd_application", "terminate_argocd_operation",
  "refresh_argocd_application", "cancel_pipeline_run", "write_cr", "delete_cr",
]);
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const preview = (command === "apply_resource" || command === "write_cr") && args?.dryRun === true ||
    (command === "helm_install" || command === "helm_upgrade") && (args?.request as { dry_run?: boolean } | undefined)?.dry_run === true;
  if (mutationCommands.has(command) && (typeof args?.context !== "string" || !args.context.trim())) throw new Error("An explicit target context is required for this action.");
  if (mutationCommands.has(command) && !preview) {
    await assertContextMutation(typeof args?.context === "string" ? args.context : undefined);
  }
  if ((command === "pod_attach_stdin" || command === "pod_attach_resize") && useUiSettings.getState().readOnly) {
    throw new Error("Global read-only mode is enabled.");
  }
  return tauriInvoke<T>(command, args);
}

export type ContextInfo = {
  name: string;
  cluster: string;
  user: string;
  namespace: string | null;
  is_current: boolean;
  is_prod: boolean;
};

export type DeletedContextSummary = {
  name: string;
  cluster: string;
  user: string;
  namespace: string | null;
  is_prod: boolean;
  deleted_at_ms: number;
  expires_at_ms: number;
  days_remaining: number;
  has_conflict: boolean;
};

export type WorkloadKind =
  | "node"
  | "namespace"
  | "event"
  | "deployment"
  | "statefulset"
  | "daemonset"
  | "replicaset"
  | "replicationcontroller"
  | "cronjob"
  | "job"
  | "pod"
  | "service"
  | "ingress"
  | "endpoint"
  | "endpointslice"
  | "configmap"
  | "secret"
  | "serviceaccount"
  | "role"
  | "rolebinding"
  | "clusterrole"
  | "clusterrolebinding"
  | "networkpolicy"
  | "persistentvolumeclaim"
  | "persistentvolume"
  | "storageclass"
  | "csidriver"
  | "csinode"
  | "volumeattributesclass"
  | "podtemplate"
  | "ingressclass"
  | "resourcequota"
  | "horizontalpodautoscaler"
  | "verticalpodautoscaler"
  | "limitrange"
  | "poddisruptionbudget"
  | "priorityclass"
  | "runtimeclass"
  | "lease"
  | "controllerrevision"
  | "mutatingwebhookconfiguration"
  | "validatingwebhookconfiguration"
  | "gatewayclass"
  | "gateway"
  | "httproute"
  | "grpcroute"
  | "jobset"
  | "apiservice"
  | "certificatesigningrequest"
  | "customresourcedefinition";

export type Health = "healthy" | "degraded" | "failed" | "unknown";

export type WorkloadSummary = {
  kind: WorkloadKind;
  name: string;
  namespace: string;
  ready: string;
  age_seconds: number;
  health: Health;
  labels: Record<string, string>;
  // Pod-only enrichment (omitted on the wire for non-pod kinds via
  // serde skip_serializing_if). Optional on the TS side.
  restart_count?: number;
  container_count?: number;
  container_ready_count?: number;
  node_name?: string;
  controlled_by?: OwnerRefLite;
  qos_class?: string;
  cpu_milli?: number;
  mem_bytes?: number;
  pod_phase?: string;
};

export type OwnerRefLite = { kind: string; name: string };

export type PodCondition = { type: string; status: string };

export type ContainerInfo = {
  name: string;
  image: string;
  ready: boolean;
  restart_count: number;
  state: string;
  cpu_request_milli: number | null;
  cpu_limit_milli: number | null;
  mem_request_bytes: number | null;
  mem_limit_bytes: number | null;
};

export type PodDetails = {
  name: string;
  namespace: string;
  status: string;
  qos_class: string;
  node_name: string | null;
  controlled_by: OwnerRefLite | null;
  service_account: string | null;
  pod_ip: string | null;
  pod_ips: string[];
  conditions: PodCondition[];
  tolerations: number;
  labels: Record<string, string>;
  annotations: Record<string, string>;
  containers: ContainerInfo[];
  cpu_usage_milli: number | null;
  mem_usage_bytes: number | null;
  age_seconds: number;
  created_at_ms: number;
};

export type ResourceDetail = {
  summary: WorkloadSummary;
  yaml: string;
  owner_refs: OwnerRefLite[];
};

export type RbacRuleDetail = {
  api_groups: string[];
  resources: string[];
  resource_names: string[];
  non_resource_urls: string[];
  verbs: string[];
};

export type RbacSubjectDetail = {
  kind: string;
  name: string;
  namespace: string | null;
};

export type RbacDetail = {
  role_ref: string | null;
  subjects: RbacSubjectDetail[];
  rules: RbacRuleDetail[];
};

export type StorageDetail = {
  phase: string | null;
  capacity: string | null;
  access_modes: string[];
  storage_class: string | null;
  volume_name: string | null;
  reclaim_policy: string | null;
  binding_mode: string | null;
  provisioner: string | null;
  allow_expansion: boolean | null;
  claim_ref: string | null;
  parameters: Record<string, string>;
};

export type ResourceInsightRow = {
  label: string;
  value: string;
};

export type ResourceInsightSection = {
  title: string;
  rows: ResourceInsightRow[];
};

export type ResourceInsights = {
  sections: ResourceInsightSection[];
};

// ─── Fleet ────────────────────────────────────────────────────────────────

export type FleetHealth = {
  pods_total: number;
  pods_ready: number;
  pods_pending: number;
  pods_failed: number;
};

export type FleetCard = {
  context: ContextInfo;
  reachable: boolean;
  error: string | null;
  server_version: string | null;
  node_count: number;
  node_ready: number;
  namespace_count: number;
  workload_count: number;
  health: FleetHealth;
  cpu_percent: number | null;
  mem_percent: number | null;
  fetched_at_ms: number;
  /**
   * Best-effort cluster distribution detected from node labels and OS image.
   * Stable lowercase id (eks / gke / aks / openshift / k3s / kind / minikube
   * / docker-desktop / rancher), or undefined when nothing matched. Optional
   * via serde skip_serializing_if on the Rust side.
   */
  distribution?: string;
};

export type NodeSummary = {
  name: string;
  roles: string[];
  version: string;
  ready: boolean;
  os_image: string;
  arch: string;
  cpu_capacity_milli: number;
  mem_capacity_bytes: number;
  pods_capacity: number;
  cpu_allocatable_milli: number;
  mem_allocatable_bytes: number;
  taints: string[];
  age_seconds: number;
  cpu_usage_milli: number | null;
  mem_usage_bytes: number | null;
  /** Mirrors `spec.unschedulable` — true when the node is cordoned. */
  unschedulable: boolean;
};

export type MetricsExplorerNode = {
  name: string;
  ready: boolean;
  cpu_allocatable_milli: number;
  mem_allocatable_bytes: number;
  cpu_usage_milli: number | null;
  mem_usage_bytes: number | null;
};

export type MetricsExplorerPod = {
  namespace: string;
  name: string;
  node_name: string | null;
  workload_kind: string;
  workload_name: string;
  cpu_usage_milli: number | null;
  mem_usage_bytes: number | null;
  cpu_request_milli: number | null;
  cpu_limit_milli: number | null;
  mem_request_bytes: number | null;
  mem_limit_bytes: number | null;
};

export type MetricsExplorerSnapshot = {
  fetched_at_ms: number;
  errors: string[];
  nodes: MetricsExplorerNode[];
  pods: MetricsExplorerPod[];
};

export type DrainFailure = {
  namespace: string;
  name: string;
  reason: string;
};

export type DrainSummary = {
  evicted: number;
  skipped_daemonset: number;
  skipped_mirror: number;
  failed: DrainFailure[];
};

// ─── CloudMap ─────────────────────────────────────────────────────────────

export type MapNodeKind =
  | "namespace"
  | "deployment"
  | "statefulset"
  | "daemonset"
  | "cronjob"
  | "job"
  | "pod"
  | "service"
  | "ingress"
  | "configmap"
  | "secret"
  | "persistent_volume_claim"
  | "node"
  | "hpa";

export type EdgeKind =
  | "selects"
  | "routes"
  | "owned_by"
  | "mounts"
  | "scales"
  | "scheduled_on";

export type MapNode = {
  id: string;
  kind: MapNodeKind;
  name: string;
  namespace: string | null;
  health: Health;
  heat: number;
  ready: string | null;
  labels: Record<string, string>;
  replicas: number | null;
  extra: Record<string, string>;
};

export type MapEdge = { from: string; to: string; kind: EdgeKind };

export type CloudMap = {
  context: string;
  nodes: MapNode[];
  edges: MapEdge[];
  fetched_at_ms: number;
};

// ─── Security ─────────────────────────────────────────────────────────────

export type Severity = "critical" | "high" | "medium" | "low" | "info";

export type Finding = {
  rule_id: string;
  title: string;
  severity: Severity;
  category: string;
  resource_kind: string;
  resource_name: string;
  namespace: string | null;
  detail: string;
  remediation: string;
};

export type SecurityReport = {
  context: string;
  findings: Finding[];
  scanned_at_ms: number;
  counts_by_severity: Record<string, number>;
  resources_scanned: number;
};

export type AccessReviewRequest = {
  kind: WorkloadKind;
  verb: string;
  namespace?: string | null;
  name?: string | null;
  subresource?: string | null;
};

export type AccessReviewResult = {
  allowed: boolean;
  denied: boolean;
  reason: string | null;
  evaluation_error: string | null;
};

// ─── CRD Browser ──────────────────────────────────────────────────────────

export type CrdSummary = {
  name: string;
  group: string;
  kind: string;
  plural: string;
  short_names: string[];
  scope: string;
  versions: string[];
  preferred_version: string;
  age_seconds: number;
};

export type CrInstance = {
  name: string;
  namespace: string | null;
  age_seconds: number;
  status_hint: string | null;
};

// ─── Activity / events stream ─────────────────────────────────────────────

/**
 * Mirrors src-tauri/src/k8s/events.rs `EventLine` — a single Kubernetes
 * Event flattened to what the activity drawer actually renders. The Rust
 * side renames `type_` (a Rust keyword conflict) so we mirror the wire
 * format here rather than the field name.
 */
export type EventLine = {
  ts: string | null;
  kind: string;
  reason: string;
  message: string;
  involved: string;
  type_: string;
};

// ─── Image vulnerability scan (C5) ────────────────────────────────────────

export type VulnSeverityCounts = {
  critical: number;
  high: number;
  medium: number;
  low: number;
  unknown: number;
};

export type VulnFinding = {
  id: string;
  package: string;
  installed_version: string;
  fixed_version: string | null;
  severity: string;
  title: string;
};

/**
 * Mirrors src-tauri/src/k8s/vulnscan.rs `VulnReport`. When trivy isn't on
 * PATH or the scan fails, `scanner_available` is false and `note` carries
 * a human-readable hint — counts/findings are empty in that case.
 */
export type VulnReport = {
  image: string;
  scanner_available: boolean;
  note: string | null;
  counts: VulnSeverityCounts;
  findings: VulnFinding[];
};

// ─── CronJob manual runs (C3) ─────────────────────────────────────────────

/**
 * Mirrors src-tauri/src/k8s/actions.rs `ManualRunSummary` — a single Job
 * created by a manual CronJob trigger, flattened to what the drawer
 * inline panel renders.
 */
export type ManualRunSummary = {
  name: string;
  status: string;
  started_at: string | null;
  completed_at: string | null;
  succeeded: number;
  failed: number;
  active: number;
};

// ─── Team Access ──────────────────────────────────────────────────────────

export type AccessTemplate = "viewer" | "editor" | "admin";

export type TokenMode = "short" | "long";

export type TeamAccessRequest = {
  member_id: string;
  namespaces: string[];
  template: AccessTemplate;
  ttl_hours: number;
  long_lived: boolean;
  service_account_namespace: string | null;
};

export type CreatedObject = {
  kind: string;
  name: string;
  namespace: string | null;
};

export type TeamAccessResult = {
  service_account: string;
  sa_namespace: string;
  token_expires_at: string;
  kubeconfig_yaml: string;
  created: CreatedObject[];
};

export type TeamGrant = {
  member_id: string;
  template: AccessTemplate | null;
  token_mode: TokenMode | null;
  service_account: string | null;
  sa_namespace: string | null;
  sa_age_seconds: number;
  cluster_wide: boolean;
  namespaces: string[];
  object_count: number;
};

// ─── Port forwards ────────────────────────────────────────────────────────

export type ForwardTargetKind = "pod" | "service";

export type ForwardSession = {
  id: string;
  context: string;
  namespace: string;
  target_kind: ForwardTargetKind;
  target_name: string;
  pod_name: string;
  local_port: number;
  remote_port: number;
  started_at_ms: number;
  bytes_in: number;
  bytes_out: number;
  connections: number;
  last_error: string | null;
};

export type PodContainerInfo = {
  name: string;
  image: string;
  is_init: boolean;
  is_default: boolean;
};

export type ApplyOutcome = {
  yaml: string;
  dry_run: boolean;
};

// ─── Helm ─────────────────────────────────────────────────────────────────

export type HelmReleaseSummary = {
  name: string;
  namespace: string;
  revision: number;
  status: string;
  chart_name: string;
  chart_version: string;
  app_version: string;
  last_deployed: string | null;
  description: string | null;
};

export type HelmReleaseDetail = {
  summary: HelmReleaseSummary;
  first_deployed: string | null;
  chart_description: string | null;
  chart_home: string | null;
  chart_icon: string | null;
  chart_sources: string[];
  chart_api_version: string | null;
  chart_type: string | null;
  chart_kube_version: string | null;
  user_values_yaml: string;
  chart_values_yaml: string;
  manifest: string;
  notes: string | null;
};

// ─── Events + actions ─────────────────────────────────────────────────────

export type EventSummary = {
  ts: string | null;
  type_: string;
  reason: string;
  message: string;
  involved_kind: string;
  involved_name: string;
  involved_uid?: string | null;
  count: number | null;
};
export type BoundedLogCapture = { text: string; truncated: boolean; bytes: number };

// ─── ArgoCD ───────────────────────────────────────────────────────────────
//
// Mirrors src-tauri/src/k8s/argocd.rs. Read-only-ish surface plus two
// mutators (sync/refresh) that go through patches on the CRD — no
// argocd CLI, no extra HTTP API.

export type ArgoApplicationSummary = {
  name: string;
  namespace: string;
  project: string;
  sync_status: string;
  health_status: string;
  repo_url: string;
  path: string;
  target_revision: string;
  destination_namespace: string;
  destination_server: string;
  age_seconds: number;
  resource_count: number;
};

export type ArgoApplicationResource = {
  group: string;
  kind: string;
  name: string;
  namespace: string | null;
  sync_status: string | null;
  health_status: string | null;
  health_message: string | null;
};

export type ArgoOperationState = {
  phase: string | null;
  message: string | null;
  started_at: string | null;
  finished_at: string | null;
  revision: string | null;
};

export type ArgoHistoryEntry = {
  revision: string;
  deployed_at: string | null;
  source_path: string | null;
};

export type ArgoApplicationDetail = {
  summary: ArgoApplicationSummary;
  resources: ArgoApplicationResource[];
  operation_state: ArgoOperationState | null;
  sync_message: string | null;
  auto_sync: boolean;
  self_heal: boolean;
  history: ArgoHistoryEntry[];
};

/** Mirrors src-tauri/src/k8s/argocd.rs `SyncOptions`. Field names use
 *  camelCase because serde-rename converts the Rust snake_case form. */
export type ArgoResourceRef = {
  group: string;
  kind: string;
  namespace?: string;
  name: string;
};
export type ArgoSyncOptions = {
  revision?: string;
  prune?: boolean;
  dryRun?: boolean;
  force?: boolean;
  replace?: boolean;
  serverSideApply?: boolean;
  applyOutOfSyncOnly?: boolean;
  respectIgnoreDifferences?: boolean;
  pruneLast?: boolean;
  retryLimit?: number | null;
  resources?: ArgoResourceRef[];
};

// ─── ArgoCD ApplicationSet + AppProject ───────────────────────────────────
//
// Both CRDs ship with the standard ArgoCD install. ApplicationSet is the
// templated-Application generator; AppProject is the project boundary.
// Read-only surfaces in v1 — sync, CRUD, role editing are clean follow-ups.

export type ArgoApplicationSetSummary = {
  name: string;
  namespace: string;
  /** First key under .spec.generators[0]: list / git / cluster / matrix / merge / ... */
  generator_kind: string | null;
  /** .spec.template.metadata.name — usually a templated string. */
  template_app_name_pattern: string | null;
  /** Number of Applications materialized, from .status.applicationStatus[]. */
  generated_count: number;
  age_seconds: number;
};

export type ArgoGeneratedApplicationRef = {
  name: string;
  namespace: string;
  /** Resolved from the matching Application CR; "Unknown" if not found. */
  sync_status: string;
  health_status: string;
};

export type ArgoApplicationSetDetail = {
  summary: ArgoApplicationSetSummary;
  /** Pretty-printed JSON-as-YAML of .spec.generators (empty if absent). */
  generators_yaml: string;
  /** Pretty-printed JSON-as-YAML of .spec.template (empty if absent). */
  template_yaml: string;
  generated_apps: ArgoGeneratedApplicationRef[];
};

export type ArgoAppProjectSummary = {
  name: string;
  /** The CR's metadata.namespace — usually `argocd`. */
  namespace: string;
  description: string;
  source_repos_count: number;
  destinations_count: number;
  cluster_resource_whitelist_count: number;
  namespace_resource_whitelist_count: number;
  age_seconds: number;
};

export type ArgoAppProjectDestination = {
  server: string;
  namespace: string;
};

export type ArgoAppProjectResourceRule = {
  group: string;
  kind: string;
};

export type ArgoAppProjectRole = {
  name: string;
  description: string;
  /** Raw Casbin policy lines, one per entry. */
  policies: string[];
};

export type ArgoAppProjectDetail = {
  summary: ArgoAppProjectSummary;
  source_repos: string[];
  destinations: ArgoAppProjectDestination[];
  cluster_resource_whitelist: ArgoAppProjectResourceRule[];
  namespace_resource_whitelist: ArgoAppProjectResourceRule[];
  roles: ArgoAppProjectRole[];
};

// ─── Tekton ───────────────────────────────────────────────────────────────
//
// Mirrors src-tauri/src/k8s/tekton.rs. Read-mostly surface plus a
// single mutator (cancel) that patches `.spec.status` on the
// PipelineRun CRD.

/** Canonical run statuses derived by the backend's `derive_run_status`.
 *  Free-form upstream condition reasons get collapsed into this closed
 *  set so the UI can pill safely. */
export type TektonRunStatus =
  | "Running"
  | "Succeeded"
  | "Failed"
  | "Cancelled"
  | "Pending"
  | "Unknown";

export type TektonPipelineRunSummary = {
  name: string;
  namespace: string;
  pipeline_ref: string | null;
  status: string;
  started_at: string | null;
  completion_time: string | null;
  duration_seconds: number | null;
  task_count: number;
  age_seconds: number;
};

export type TektonTaskRunStatus = {
  name: string;
  display_name: string | null;
  status: string;
  started_at: string | null;
  completion_time: string | null;
  duration_seconds: number | null;
  message: string | null;
};

export type TektonConditionEntry = {
  type: string;
  status: string;
  reason: string | null;
  message: string | null;
};

export type TektonPipelineRunDetail = {
  summary: TektonPipelineRunSummary;
  conditions: TektonConditionEntry[];
  tasks: TektonTaskRunStatus[];
  /** [name, value] pairs from `.spec.params`. Array / object values are JSON-encoded. */
  params: [string, string][];
  workspaces: string[];
  /** True when the v1 per-TaskRun fan-out hit the backend cap and `tasks`
   *  is the first N (currently 50) of a larger child set. The detail
   *  panel surfaces this so users know they're not seeing every task. */
  tasks_truncated: boolean;
};

// ─── API ──────────────────────────────────────────────────────────────────

type MutationAuditInput = Omit<ChangeHistoryEventInput, "status" | "error">;

function auditTarget(
  context: string | undefined,
  namespace: string | undefined,
  kind: string,
  name: string,
): ChangeHistoryTarget {
  return {
    context: context ?? "",
    namespace: namespace ?? "",
    kind,
    name,
  };
}

async function trackMutation<T>(
  input: MutationAuditInput,
  run: () => Promise<T>,
): Promise<T> {
  try {
    const result = await run();
    useChangeHistoryStore.getState().recordEvent({
      ...input,
      status: "success",
    });
    return result;
  } catch (error) {
    useChangeHistoryStore.getState().recordEvent({
      ...input,
      status: "failure",
      error,
    });
    throw error;
  }
}

function argoAction(options: ArgoSyncOptions): ChangeHistoryAction {
  return options.revision && options.prune ? "argocd-rollback" : "argocd-sync";
}

export const k8s = {
  listContexts: () => invoke<ContextInfo[]>("list_contexts"),
  setContext: (name: string) => invoke<ContextInfo>("set_context", { name }),
  deleteContext: (name: string) => invoke<void>("delete_context", { name }),
  listDeletedContexts: () =>
    invoke<DeletedContextSummary[]>("list_deleted_contexts"),
  restoreDeletedContext: (name: string, overwrite = false) =>
    invoke<void>("restore_deleted_context", { name, overwrite }),
  listNamespaces: (context?: string) =>
    invoke<string[]>("list_namespaces", { context }),
  listWorkloads: (namespace: string, kind: WorkloadKind, context?: string) =>
    invoke<WorkloadSummary[]>("list_workloads", { namespace, kind, context }),
  getResource: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) => invoke<ResourceDetail>("get_resource", { namespace, kind, name, context }),
  listPodsFor: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) =>
    invoke<WorkloadSummary[]>("list_pods_for", { namespace, kind, name, context }),
  listPodsOnNode: (node: string, context?: string) =>
    invoke<WorkloadSummary[]>("list_pods_on_node", { node, context }),
  getPodDetails: (ctx: string, namespace: string, name: string) =>
    invoke<PodDetails>("get_pod_details", { ctx, namespace, name }),
  listFleet: () => invoke<FleetCard[]>("list_fleet"),
  probeFleetContext: (context: string) =>
    invoke<FleetCard>("probe_fleet_context", { context }),
  disconnectContext: (context: string) =>
    invoke<void>("disconnect_context", { context }),
  reconnectAll: () => invoke<void>("reconnect_all"),
  listNodes: (context?: string) =>
    invoke<NodeSummary[]>("list_nodes", { context }),
  metricsExplorerSnapshot: (namespace?: string, context?: string) =>
    invoke<MetricsExplorerSnapshot>("metrics_explorer_snapshot", {
      namespace,
      context,
    }),
  cloudMap: (context?: string, namespace?: string) =>
    invoke<CloudMap>("cloud_map", { context, namespace }),
  networkDebugSnapshot: (namespace: string, context?: string) =>
    invoke<NetworkDebugSnapshot>("network_debug_snapshot", { namespace, context }),
  securityScan: (context?: string) =>
    invoke<SecurityReport>("security_scan", { context }),
  checkAccess: (request: AccessReviewRequest, context?: string) =>
    invoke<AccessReviewResult>("check_access", { request, context }),
  getRbacDetails: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) => invoke<RbacDetail>("get_rbac_details", { namespace, kind, name, context }),
  getStorageDetails: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) =>
    invoke<StorageDetail>("get_storage_details", {
      namespace,
      kind,
      name,
      context,
    }),
  getResourceInsights: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) =>
    invoke<ResourceInsights>("get_resource_insights", {
      namespace,
      kind,
      name,
      context,
    }),
  listCrds: (context?: string) => invoke<CrdSummary[]>("list_crds", { context }),
  listCrInstances: (
    group: string,
    version: string,
    kind: string,
    plural: string,
    namespace?: string,
    context?: string,
  ) =>
    invoke<CrInstance[]>("list_cr_instances", {
      group,
      version,
      kind,
      plural,
      namespace,
      context,
    }),
  getCrYaml: (
    group: string,
    version: string,
    kind: string,
    plural: string,
    name: string,
    namespace?: string,
    context?: string,
  ) =>
    invoke<string>("get_cr_yaml", {
      group,
      version,
      kind,
      plural,
      name,
      namespace,
      context,
    }),
  provisionTeamAccess: (request: TeamAccessRequest, context?: string) =>
    invoke<TeamAccessResult>("provision_team_access", { request, context }),
  revokeTeamAccess: (
    memberId: string,
    namespaces: string[],
    context?: string,
  ) =>
    invoke<CreatedObject[]>("revoke_team_access", {
      memberId,
      namespaces,
      context,
    }),
  listTeamAccess: (context?: string) =>
    invoke<TeamGrant[]>("list_team_access", { context }),
  renewTeamToken: (memberId: string, ttlHours: number, context?: string) =>
    invoke<TeamAccessResult>("renew_team_token", {
      memberId,
      ttlHours,
      context,
    }),
  rotateTeamToken: (memberId: string, context?: string) =>
    invoke<TeamAccessResult>("rotate_team_token", { memberId, context }),
  listEventsFor: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) =>
    invoke<EventSummary[]>("list_events_for", {
      namespace,
      kind,
      name,
      context,
    }),
  captureIncidentLogs: (context: string, namespace: string, pod: string, expectedUid: string, container: string, previous: boolean) =>
    invoke<BoundedLogCapture>("capture_incident_logs", { context, expectedUid, selector: { namespace, pod_name: pod, container, previous, tail_lines: 201, label_selector: null, since_seconds: null } }),
  restartWorkload: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) =>
    trackMutation(
      {
        action: "restart",
        target: auditTarget(context, namespace, kind, name),
        summary: `restart requested for ${kind}/${name}`,
      },
      () => invoke<void>("restart_workload", { namespace, kind, name, context }),
    ),
  scaleWorkload: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    replicas: number,
    context?: string,
  ) =>
    trackMutation(
      {
        action: "scale",
        target: auditTarget(context, namespace, kind, name),
        summary: `scaled ${kind}/${name} to ${replicas}`,
        details: { replicas },
      },
      () =>
        invoke<void>("scale_workload", {
          namespace,
          kind,
          name,
          replicas,
          context,
        }),
    ),
  /** Returns true when `trivy --version` succeeds; surfaced by the UI to gate
   *  the scan button without round-tripping through scan_image. */
  detectTrivy: () => invoke<boolean>("detect_trivy"),
  /** Run trivy against a single image. Always resolves with a structured
   *  report — failure paths set scanner_available=false and put the reason
   *  in `note` rather than throwing. */
  scanImage: (image: string) => invoke<VulnReport>("scan_image", { image }),
  /**
   * List Jobs that were created by manual triggers of a CronJob — used by
   * the drawer's "Manual runs" section. Filtered server-side by the
   * cronjob.kubernetes.io/instantiate=manual label and by ownerReference.
   */
  listManualCronjobRuns: (namespace: string, name: string, context?: string) =>
    invoke<ManualRunSummary[]>("list_manual_cronjob_runs", {
      namespace,
      name,
      context,
    }),
  /**
   * Hot-swap a container's image on a Deployment / StatefulSet / DaemonSet —
   * equivalent to `kubectl set image <kind>/<name> <container>=<image>`.
   * Triggers a normal rolling update through the controller.
   */
  setWorkloadImage: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    container: string,
    image: string,
    context?: string,
  ) =>
    trackMutation(
      {
        action: "set-image",
        target: auditTarget(context, namespace, kind, name),
        summary: `set ${container} image on ${kind}/${name}`,
        details: { container, image },
      },
      () =>
        invoke<void>("set_workload_image", {
          namespace,
          kind,
          name,
          container,
          image,
          context,
        }),
    ),
  deletePod: (namespace: string, name: string, context?: string) =>
    trackMutation(
      {
        action: "delete",
        target: auditTarget(context, namespace, "pod", name),
        summary: `deleted pod/${name}`,
      },
      () => invoke<void>("delete_pod", { namespace, name, context }),
    ),
  /** Mark a node unschedulable. New pods won't be placed on it; existing pods stay. */
  cordonNode: (name: string, context?: string) =>
    invoke<void>("cordon_node", { name, context }),
  /** Clear `spec.unschedulable` so the scheduler resumes placing pods. */
  uncordonNode: (name: string, context?: string) =>
    invoke<void>("uncordon_node", { name, context }),
  /**
   * Cordon then evict all non-DaemonSet, non-mirror pods from the node.
   * Returns a per-pod summary; PDB-blocked pods land in `failed` rather
   * than being force-deleted.
   */
  drainNode: (name: string, context?: string) =>
    invoke<DrainSummary>("drain_node", { name, context }),
  /**
   * Manually trigger a CronJob — equivalent to `kubectl create job
   * --from=cronjob/<name>`. Resolves to the name of the freshly-created Job.
   */
  triggerCronjob: (namespace: string, name: string, context?: string) =>
    trackMutation(
      {
        action: "trigger",
        target: auditTarget(context, namespace, "cronjob", name),
        summary: `triggered cronjob/${name}`,
      },
      () => invoke<string>("trigger_cronjob", { namespace, name, context }),
    ),
  deleteResource: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    context?: string,
  ) =>
    trackMutation(
      {
        action: "delete",
        target: auditTarget(context, namespace, kind, name),
        summary: `deleted ${kind}/${name}`,
      },
      () => invoke<void>("delete_resource", { namespace, kind, name, context }),
    ),
  listPodContainers: (namespace: string, pod: string, context?: string) =>
    invoke<PodContainerInfo[]>("list_pod_containers", { namespace, pod, context }),
  startPortForward: (opts: {
    namespace: string;
    targetKind: ForwardTargetKind;
    targetName: string;
    localPort: number;
    remotePort: number;
    context?: string;
  }) =>
    invoke<ForwardSession>("start_port_forward", {
      namespace: opts.namespace,
      targetKind: opts.targetKind,
      targetName: opts.targetName,
      localPort: opts.localPort,
      remotePort: opts.remotePort,
      context: opts.context,
    }),
  listPortForwards: () => invoke<ForwardSession[]>("list_port_forwards"),
  stopPortForward: (id: string) => invoke<boolean>("stop_port_forward", { id }),
  applyResource: (
    namespace: string,
    kind: WorkloadKind,
    name: string,
    yaml: string,
    dryRun: boolean,
    context?: string,
  ) =>
    dryRun
      ? invoke<ApplyOutcome>("apply_resource", {
          namespace,
          kind,
          name,
          yaml,
          dryRun,
          context,
        })
      : trackMutation(
          {
            action: "apply",
            target: auditTarget(context, namespace, kind, name),
            summary: `applied ${kind}/${name}`,
          },
          () =>
            invoke<ApplyOutcome>("apply_resource", {
              namespace,
              kind,
              name,
              yaml,
              dryRun,
              context,
            }),
        ),
  listHelmReleases: (context?: string) =>
    invoke<HelmReleaseSummary[]>("list_helm_releases", { context }),
  getHelmRelease: (
    namespace: string,
    name: string,
    revision?: number,
    context?: string,
  ) =>
    invoke<HelmReleaseDetail>("get_helm_release", {
      namespace,
      name,
      revision: revision ?? null,
      context,
    }),
  listHelmHistory: (namespace: string, name: string, context?: string) =>
    invoke<HelmReleaseSummary[]>("list_helm_history", { namespace, name, context }),
  helmInstall: (
    request: HelmInstallRequest,
    streamId: string,
    channel: import("@tauri-apps/api/core").Channel<HelmEvent>,
    context?: string,
  ) =>
    invoke<void>("helm_install", { request, streamId, channel, context }),
  helmUpgrade: (
    request: HelmUpgradeRequest,
    streamId: string,
    channel: import("@tauri-apps/api/core").Channel<HelmEvent>,
    context?: string,
  ) =>
    invoke<void>("helm_upgrade", { request, streamId, channel, context }),
  helmRollback: (
    request: HelmRollbackRequest,
    streamId: string,
    channel: import("@tauri-apps/api/core").Channel<HelmEvent>,
    context?: string,
  ) =>
    invoke<void>("helm_rollback", { request, streamId, channel, context }),
  helmUninstall: (
    request: HelmUninstallRequest,
    streamId: string,
    channel: import("@tauri-apps/api/core").Channel<HelmEvent>,
    context?: string,
  ) =>
    invoke<void>("helm_uninstall", { request, streamId, channel, context }),
  helmSearchRepo: (query: string) =>
    invoke<HelmChartHit[]>("helm_search_repo", { query }),
  helmShowValues: (chart: string, version?: string) =>
    invoke<string>("helm_show_values", { chart, version: version ?? null }),
  getDebugTarget: (context: string, namespace: string, pod: string) =>
    invoke<DebugTarget>("get_debug_target", { context, namespace, pod }),
  createDebugContainer: (request: DebugRequest) =>
    invoke<DebugResult>("create_debug_container", { request, context: request.context }),
  // Pod attach commands. See PodTerminal for end-to-end usage.
  startPodAttach: (
    request: {
      namespace: string;
      pod: string;
      pod_uid?: string;
      container: string | null;
      command: string[];
      tty: boolean;
      cols?: number;
      rows?: number;
    },
    channel: import("@tauri-apps/api/core").Channel<AttachEvent>,
    context?: string,
  ) =>
    invoke<string>("start_pod_attach", {
      request,
      channel,
      context,
    }),
  podAttachStdin: (id: string, bytes: number[]) =>
    invoke<void>("pod_attach_stdin", { id, bytes }),
  podAttachResize: (id: string, cols: number, rows: number) =>
    invoke<void>("pod_attach_resize", { id, cols, rows }),
  podAttachClose: (id: string) => invoke<void>("pod_attach_close", { id }),
  // ── ArgoCD ────────────────────────────────────────────────────────────
  /** Cheap probe — true when the cluster has the Application CRD registered. */
  detectArgocd: (context?: string) =>
    invoke<boolean>("detect_argocd", { context }),
  listArgocdApplications: (context?: string, namespace?: string) =>
    invoke<ArgoApplicationSummary[]>("list_argocd_applications", {
      context,
      namespace,
    }),
  getArgocdApplication: (
    context: string | undefined,
    namespace: string,
    name: string,
  ) =>
    invoke<ArgoApplicationDetail>("get_argocd_application", {
      context,
      namespace,
      name,
    }),
  /** Trigger a sync. Sets `.operation.sync` on the Application CRD —
   *  the ArgoCD controller picks up the operation and reconciles.
   *  See {@link ArgoSyncOptions} for the full set of canonical flags. */
  syncArgocdApplication: (
    context: string | undefined,
    namespace: string,
    name: string,
    options: ArgoSyncOptions,
  ) =>
    trackMutation(
      {
        action: argoAction(options),
        target: auditTarget(context, namespace, "argocdapplication", name),
        summary:
          argoAction(options) === "argocd-rollback"
            ? `argocd rollback ${name}`
            : `argocd sync ${name}`,
        details: {
          prune: options.prune ?? false,
          dryRun: options.dryRun ?? false,
          revision: options.revision ?? "",
          resources: options.resources?.length ?? 0,
        },
      },
      () =>
        invoke<void>("sync_argocd_application", {
          context,
          namespace,
          name,
          options,
        }),
    ),
  /** Cancel an in-flight sync operation. Patches `.operation` to null —
   *  same path as the upstream ArgoCD UI's "Terminate" button. */
  terminateArgocdOperation: (
    context: string | undefined,
    namespace: string,
    name: string,
  ) =>
    trackMutation(
      {
        action: "argocd-terminate",
        target: auditTarget(context, namespace, "argocdapplication", name),
        summary: `argocd terminate ${name}`,
      },
      () =>
        invoke<void>("terminate_argocd_operation", {
          context,
          namespace,
          name,
        }),
    ),
  /** Annotation-based refresh. `hard=true` re-clones the repo. */
  refreshArgocdApplication: (
    context: string | undefined,
    namespace: string,
    name: string,
    hard: boolean,
  ) =>
    invoke<void>("refresh_argocd_application", {
      context,
      namespace,
      name,
      hard,
    }),
  /** Cheap probe — true when the cluster has the ApplicationSet CRD.
   *  Usually redundant with `detectArgocd` since the CRD ships in the
   *  same install; exposed so tabs can be hidden cleanly when a custom
   *  install strips it out. */
  detectArgocdApplicationSets: (context?: string) =>
    invoke<boolean>("detect_argocd_application_sets", { context }),
  listArgocdApplicationSets: (context?: string, namespace?: string) =>
    invoke<ArgoApplicationSetSummary[]>("list_argocd_application_sets", {
      context,
      namespace,
    }),
  getArgocdApplicationSet: (
    context: string | undefined,
    namespace: string,
    name: string,
  ) =>
    invoke<ArgoApplicationSetDetail>("get_argocd_application_set", {
      context,
      namespace,
      name,
    }),
  listArgocdAppProjects: (context?: string) =>
    invoke<ArgoAppProjectSummary[]>("list_argocd_app_projects", { context }),
  getArgocdAppProject: (
    context: string | undefined,
    namespace: string,
    name: string,
  ) =>
    invoke<ArgoAppProjectDetail>("get_argocd_app_project", {
      context,
      namespace,
      name,
    }),
  // ── Tekton ────────────────────────────────────────────────────────────
  /** Cheap probe — true when the cluster registers the PipelineRun CRD
   *  in either v1 or v1beta1. */
  detectTekton: (context?: string) =>
    invoke<boolean>("detect_tekton", { context }),
  listPipelineRuns: (context?: string, namespace?: string) =>
    invoke<TektonPipelineRunSummary[]>("list_pipeline_runs", {
      context,
      namespace,
    }),
  getPipelineRun: (
    context: string | undefined,
    namespace: string,
    name: string,
  ) =>
    invoke<TektonPipelineRunDetail>("get_pipeline_run", {
      context,
      namespace,
      name,
    }),
  /** Cancel a running PipelineRun. Patches `.spec.status: "Cancelled"`
   *  and writes the legacy `tekton.dev/status` annotation in the same
   *  body so older Tekton controllers also pick up the signal. */
  cancelPipelineRun: (
    context: string | undefined,
    namespace: string,
    name: string,
  ) =>
    invoke<void>("cancel_pipeline_run", {
      context,
      namespace,
      name,
    }),
};

// ─── Helm write ops (CLI shell-out) ───────────────────────────────────────

export type HelmEvent =
  | { kind: "stdout"; line: string }
  | { kind: "stderr"; line: string }
  | { kind: "exited"; code: number }
  | { kind: "error"; message: string };

export type HelmInstallRequest = {
  release: string;
  chart: string;
  namespace: string;
  version: string | null;
  values_yaml: string | null;
  create_namespace: boolean;
  wait: boolean;
  dry_run?: boolean;
};

export type HelmUpgradeRequest = {
  release: string;
  chart: string;
  namespace: string;
  version: string | null;
  values_yaml: string | null;
  install: boolean;
  wait: boolean;
  atomic: boolean;
  dry_run?: boolean;
};

export type HelmRollbackRequest = {
  release: string;
  namespace: string;
  revision: number;
  wait: boolean;
  dry_run?: boolean;
};

export type HelmUninstallRequest = {
  release: string;
  namespace: string;
  keep_history: boolean;
};

export type HelmChartHit = {
  name: string;
  version: string;
  app_version: string;
  description: string;
};

// ─── Watches ──────────────────────────────────────────────────────────────
//
// Watches stream `WatchEvent<T>`s through a Tauri Channel. The frontend only
// uses them as cache-invalidation triggers today (not as the data source) —
// the existing `useQuery` keeps working and simply re-runs when the watch
// reports an Apply/Delete. Cancellation is via `stop_stream` with the same
// `streamId` used to open the watch.

export type WatchEvent<T> =
  | { kind: "applied"; item: T }
  | { kind: "deleted"; name: string }
  | { kind: "init_done" }
  | { kind: "error"; message: string };

// ─── Pod attach (terminal) ────────────────────────────────────────────────

export type AttachEvent =
  | { kind: "stdout"; text: string }
  | { kind: "stderr"; text: string }
  | { kind: "closed"; message: string | null; exit_code: number | null };
