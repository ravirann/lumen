# GPU scheduling, allocation, and usage design

Date: October 2, 2026. Inspected baseline: `4606411`.
Status: proposed design awaiting user review.
Research: [GPU and AI feature research](2026-10-02-gpu-ai-feature-research.md).

## Intent and scope

The user selected teams running GPU and AI workloads and approved proceeding
with the research recommendation. This design covers the first two priorities:
explain waiting GPU work and connect allocation to optional measured usage.
Ray, Kubeflow, KServe, and inference-specific views are subsequent work, chosen
from pilot environments rather than bundled into this implementation.

Success means an operator can select a waiting pod, see the available explanation
and supporting objects, inspect its requested or allocated GPU resources, and
view usage history when their cluster already exposes suitable telemetry.
The app remains local, uses the selected kubeconfig context, and requires no new
Lumen cluster agent or hosted service.

## Approach and alternatives

Recommended: reuse triage, device resources, namespace scope, and incident
investigation; add bounded native readers and small typed adapters. Use an
existing in-cluster Prometheus service through the Kubernetes API service proxy
for the first telemetry integration. This reuses kubeconfig authentication and
avoids storing separate telemetry credentials.

A telemetry-first GPU dashboard would expose usage sooner but would leave
clusters without telemetry unsupported and would not explain queue admission.
A full ML platform console would add many controller-specific lifecycles before
the shared scheduling and device foundation is dependable. Neither is selected.

## User flow

1. A Pending GPU pod in triage offers **Explain scheduling**. The investigation
   shows separate admission, scheduling, and startup sections, with evidence
   and links to related resources. A pod waiting on model download after being
   scheduled is not labelled as lacking GPU capacity.
2. The device resources page gains **GPU workloads** and **GPU nodes** views
   alongside its current claims/templates/classes/slices views. Pod and node
   detail links preserve context and namespace. Existing DRA views remain usable
   when GPU extended resources or telemetry are absent.
3. **Usage history** offers setup when no source is configured. Users explicitly
   enter an existing Prometheus Service namespace, name, and port for this
   context, test the connection, and select a time window. No automatic exporter
   installation, service discovery scan, or metrics collection begins at launch.

Use existing page, table, source-notice, and investigation components. Present
timestamps, unavailable-source messages, and metric definitions near their data.
Reuse the current incident report preview/redaction flow for scheduling evidence.
GPU views are read-only; existing workload actions retain their existing gates.

## Increment A: scheduling explanation

Add an explicit-context native command taking namespace, pod name, and expected
UID. Fetch the current pod, UID-matched events, relevant PVCs, node scheduling
summaries, and referenced DRA claims. Collect sources independently. Verify the
pod UID before presenting an explanation; replacement invalidates the selected
investigation rather than attaching evidence to the new pod.

The result distinguishes `observed` controller/event statements, `inferred`
configuration explanations, and `unknown` outcomes. Each explanation carries
source kind/name/UID where available, capture time, and any controller condition
transition time or observedGeneration. Collection is not an atomic cluster snapshot.

Supported initial explanations:

- Kubernetes scheduler-reported insufficient resources, untolerated taints,
  node selector/affinity mismatch, and storage binding conflicts.
- Pending DRA claims or unresolved claim references, using existing claim logic.
- Scheduled pods blocked during startup, using container/init-container state.
- Optional Kueue quota reservation and admission-check conditions.

Retain the original scheduler message when it contains multiple constraints;
do not manufacture an exhaustive node eligibility matrix. Static analysis may
explain a selector or request but does not reproduce scheduler plugins,
preemption, topology-aware placement, or cloud capacity provisioning.

For Kueue, discover served API versions and use known adapters for v1beta1 and
v1beta2. Resolve Workload membership through verified owner references to the
pod's supported Job/controller ancestry, not matching names. Read the selected
namespace's Workloads and referenced LocalQueue; optionally read its ClusterQueue,
ResourceFlavor, and AdmissionCheck. Unsupported version or unresolved ownership
means unavailable Kueue explanation, while ordinary pod evidence still works.
Bound owner traversal to four hops, verify each referenced UID, and detect cycles.
Fetch only directly referenced supporting objects rather than recursively loading
every queue, cohort, or controller relationship.
Do not show a predicted start time or imply that quota reservation guarantees
pod scheduling. Queue position is outside the first version.

## Increment B: GPU requests and allocation visibility

Add a GPU inventory reader without widening the existing sanitized DRA pod
projection. Return purpose-specific node and pod summaries: UID, namespace,
name, phase, node, owner references, scheduling fields needed by the explainer,
and accelerator resource quantities. Do not return arbitrary env values,
annotations, opaque driver configuration, or Secret bodies.

Initial extended-resource adapters cover `nvidia.com/gpu`,
`nvidia.com/gpu.shared`, and NVIDIA MIG resource names. Keep the model extensible,
but do not claim tested AMD or other vendor-specific health/telemetry support.
Calculate effective pod requests using Kubernetes application/init-container
semantics, including restartable init containers and overhead where applicable.
Unsupported request forms yield unknown accounting rather than a guessed sum.
Show requested resources separately from node advertised allocatable resources.
Exclude terminal pods from current requested-resource aggregates.

Never translate advertised shared slots directly into physical GPU counts.
GPU product/MIG/sharing labels are reported metadata, not hardware measurements.
For device-plugin workloads, a scheduled pod establishes a node relationship
but not an exact GPU UUID. Exact device identity requires explicit exporter or
driver mapping. For DRA, preserve driver/pool/device identities and existing
claim-to-pod resolution without adding them to legacy GPU slot totals.

With namespace-scoped pod visibility, totals mean observed namespace requests.
No cluster-wide free-device number is shown. Physical device, MIG instance,
advertised slot, requested amount, and allocation remain distinct concepts.

## Increment C: optional historical telemetry

Configuration is keyed by cluster context: Service namespace/name/port and the
fixed Prometheus API path, plus an optional exact cluster-label selector for
multi-cluster telemetry. Validate the selector as a label name and literal value;
do not accept raw PromQL. When a source has no cluster label, users explicitly
declare that it contains only this cluster; show that as an unverified source
assumption rather than discovered provenance. Persist non-secret configuration through a small
native settings module. Use the selected context's Kubernetes client for
service-proxy GET requests; no bearer tokens, arbitrary URL, custom headers, or
TLS verification bypass are accepted. Kubernetes enforces services/proxy access;
preflight and failures explain missing permission.

Service-proxy access can expose the configured Prometheus service's dataset.
Namespace filters scope Lumen's query, not authorization inside Prometheus.
Show that distinction during source setup. Failed proxy support does not trigger
a silent fallback to a different authentication or network path.

Use fixed query families for available GPU utilization, framebuffer memory,
supported profiling activity, and hardware-error signals. Fetch existing
DCGM series through Prometheus; do not scrape and store telemetry locally.
Offer 1 hour, 6 hours, 24 hours, and 7 days, with at most 1,000 returned points per
series, 200 series, 4 MiB response size, and a 15-second request deadline.
Reject an oversized result with a narrower-query instruction. Native responses
contain only allowlisted metrics and identity labels; no unrestricted PromQL UI.

Match cluster, device/MIG identity, namespace, pod UID where available, and
container labels. Missing cluster separation or ambiguous identity prevents
workload attribution. Pod-name-only series may be displayed as unverified
associations, never as evidence for an exact pod instance. Historical device
series remain visible after a device or pod disappears from current inventory.

Exporter versions, enabled counters, sharing modes, and labels vary. Capability
inspection reports available metric families and usable identity labels; absent
families remain unavailable. A physically shared device's utilization must not
be duplicated as exclusive utilization for every consuming pod. Show device
level data when per-workload attribution cannot be established. No optimization
recommendations or monetary savings calculations derive from this integration.

Usage requests occur on entry, time-window change, or explicit refresh. Stop
polling on navigation away; no new background scrape loop. Query keys include
context, source, scope, identity, and time window. Source/context changes discard
pending responses and clear incompatible cached results.

## Native and frontend boundaries

- Existing foundations: `src/lib/deviceResources.ts`,
  `src/routes/cluster/DeviceResourcesView.tsx`,
  `src/components/TriageInvestigation.tsx`, `src/lib/triage.ts`,
  `src-tauri/src/k8s/devices.rs`, and `src-tauri/src/commands/devices.rs`.
- Proposed native modules: `k8s/gpu_inventory.rs`, `k8s/gpu_scheduling.rs`,
  `k8s/kueue.rs`, `k8s/gpu_telemetry.rs`, and matching command registration.
- Proposed frontend modules: typed GPU evidence contracts, pure scheduling and
  attribution helpers, GPU tables, and a usage-history panel. Reuse namespace
  scope and incident reporting instead of creating another workspace subsystem.

Commands require an explicit non-empty context. Namespace, resource identity,
source configuration, time ranges, and response limits are validated natively.
Kubernetes readers use 500-item pages, a 10,000-item per-source cap, and
15-second source deadlines, with client setup bounded separately. Source contracts
include state, completeness, capture time, and a sanitized message. Denial, API
absence, timeout, empty results, and truncation are distinct. A failed refresh
does not allow cached evidence to establish a new scheduling conclusion.

## Validation and completion criteria

Meaningful pure tests cover request accounting, shared-slot interpretation,
identity matching, source completeness, and explanation confidence. Native mock
API tests cover pagination, timeouts, API version selection, UID replacement,
RBAC denial, redaction, service-proxy paths, and bounded telemetry responses.

Use kind fixtures for namespace-only permissions, GPU request metadata, pending
PVCs, scheduler events, DRA references, and both Kueue schema adapters. Synthetic
GPU resources demonstrate UI/accounting only, not real device allocation.
Frontend tests exercise stale-refresh behavior, context switching during requests,
no-telemetry setup, ambiguous metric attribution, and selected-issue export.

Before claiming hardware compatibility, run a real NVIDIA pilot covering
dedicated devices, a supported MIG configuration, and a supported sharing
configuration. Missing pilot access does not block implementation or fixture
verification, but the release notes must identify hardware validation as pending.
Observe an operator explaining a waiting job, identifying its allocation, and
investigating low throughput; record terminal workarounds and wrong conclusions.

Run repository lint, frontend tests/build/bundle budgets, Rust tests/fmt/Clippy,
and the appropriate kind integration tests. Deliver each increment independently
with capability-dependent UI and documented support boundaries. Do not publish
a release or push remote source as part of design approval.

## Explicit exclusions

This work does not create a scheduler, install drivers/exporters, submit training
jobs, change GPU sharing configuration, run automatic remediation, collect model
prompts, or build an experiment tracker. Active NCCL/fabric diagnostics, cloud
quota integrations, Ray/Kubeflow/KServe adapters, and vLLM-specific metrics are
separate follow-up designs.
