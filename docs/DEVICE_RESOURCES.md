# Device resources

Open **device resources** in the cluster navigation to inspect Kubernetes Dynamic
Resource Allocation (DRA). Pod details link to their claims and reported health;
node details link to device inventory.

## Requirements and scope

Lumen reads `resource.k8s.io/v1`, the stable DRA API introduced in Kubernetes 1.34.
It does not fall back to beta APIs. A DRA driver must publish resources before
hardware inventory appears. This feature does not install drivers or create,
modify, or delete device resources.

The namespace selection applies to ResourceClaims, ResourceClaimTemplates, and
Pods. DeviceClasses and ResourceSlices are cluster-scoped. Each source uses the
permissions of the selected kubeconfig context. A user who can list claims in
one namespace can still inspect those claims when cluster-scoped reads are denied.
Enter a known namespace in the namespace picker if namespace discovery is denied.

API absence, denied access, request errors, and a successful empty list have
different messages. Successful sources remain usable if another source fails.
Refresh takes a new observation; the displayed capture time is not a driver
health-update timestamp. The independent list operations are not an atomic
snapshot across all objects.

## Claims, inventory, and health

Claims show allocated device identities. For a pod, Lumen resolves direct claim
references or generated claim names from pod status, then locates devices using
the driver, pool, and device tuple. Missing claims, pending allocations, and
unavailable inventory are distinct states.

Inventory uses the latest observed generation of each driver pool. Its expected
slice count determines whether the pool observation is complete. Node-name and
all-node placement can establish node access; selector-based placement remains
unknown when the required node labels are unavailable. A published device is not
necessarily unused. Sharing, administrative allocations, incomplete data, and
restricted namespace visibility prevent reliable global free-device counts.

Health comes from matching per-container Kubernetes status reports. Whole-claim
and individual-request reports are supported. A consuming container without a
matching report is shown as unknown. Terminated pods can retain old reports.
DRA health inspection does not probe hardware, infer health from pod readiness,
or measure GPU utilization. Optional exporter history is described below. Driver support and cluster feature configuration determine which
health reports exist.

The inspector displays sanitized JSON, which is also valid YAML syntax. It
removes annotations and managed fields, and redacts opaque configuration and
arbitrary driver status data. This inspection view is not an export for replaying
the complete original manifest.

## Bounds and verification

Native requests paginate at 500 objects per page, with a 10,000-object cap per
source and a 15-second source deadline. Discovery and client setup have separate
deadlines. A truncated or failed source is never presented as a complete inventory.

Unit tests exercise API discovery, authorization, pagination, deadlines,
redaction, claim relationships, health matching, and UI source states. The
`devices_cluster` integration test runs through `scripts/test-kind.sh` using a
disposable cluster and an isolated kubeconfig. It verifies real API schemas,
namespace isolation, read-only RBAC and sanitized responses with synthetic device
resources. It does not establish hardware allocation or real driver health behavior.

## YAML draft review

The shared YAML editor offers **Review changes**, comparing the edit-start
snapshot with the unsubmitted draft. **Revert draft** restores that snapshot.
Any text change invalidates earlier dry-run validation. If freshly loaded resource
data differs during editing, the draft is preserved and **Reload latest** explicitly
discards it before further apply attempts. Server dry-run output remains separately
visible so admission changes are distinguishable from typed edits.

Revert is a local editing action, not cluster rollback. Dry-run does not lock a
resource against another writer. Existing RBAC, protected-context, confirmation,
and read-only controls continue to apply. For oversized documents, line comparison
shows an explicit unavailable notice; the complete draft remains editable.

## GPU workloads and nodes

**GPU allocation** displays independent **GPU workloads** and **GPU nodes**
views even when DRA APIs are absent. Effective active pod requests are observed
namespace-scoped totals, with partial sources and unknown quantities visible.
Terminal pods are excluded. NVIDIA GPU, shared and MIG resource keys stay
separate from cluster-wide advertised allocatable slots and DRA driver/pool/device
identities. Shared slots are not physical device counts; no free capacity is
inferred. A scheduled workload establishes its node relationship, not a GPU UUID.
Pod links preserve context and namespace; DRA claim and node inventory links
open the existing device inspection. Reported product/MIG/sharing labels are
metadata, not hardware measurements.

## Optional usage history

Select **Usage history** to configure an existing Prometheus Service for the
selected context. Enter Service namespace, name and port, then either an exact
cluster label/value or an explicit single-cluster acknowledgement. Save the
source before **Test saved source**; **Remove source** clears its configuration
and incompatible history. Native settings persist only non-secret source fields.
No separate credentials, arbitrary URL, PromQL editor, TLS bypass or exporter
installation is supported.

Kubeconfig credentials access the Service through Kubernetes `services/proxy`
GET. This permission can expose the Service's entire Prometheus dataset;
namespace filtering scopes Lumen's queries, not authorization inside Prometheus.
There is no alternate network/authentication fallback. Connection and persistence
failures show fixed messages without raw server or credential details.

Capability and identity-label coverage appear before charts. Fixed DCGM families
include utilization, framebuffer used/total, SM activity, tensor activity and XID
signals where available. Utilization and profiling values are percentages;
framebuffer values are bytes. XID values are observations, not error counts or
root-cause diagnoses. Missing counters remain unavailable; nonfinite readings
stay gaps instead of becoming idle zeroes. SVG display is downsampled while native
measurement points remain intact.

Choose 1 hour, 6 hours, 24 hours or 7 days. Each native history operation is
bounded to 1,000 points per series, 200 series, 4 MiB and 15 seconds. Oversized
responses fail with narrower-window/scope guidance. Usage requests start on
entry, window changes or explicit refresh; no timer polling runs after
navigation away. Context, serialized source, namespace, window and end time
isolate query results. Failed refresh hides previous readings until a successful
refresh, and pending results from an old source/context never replace current
history.

Device history stays visible when live pods, nodes or DRA mappings are absent,
including device series without a namespace. Selecting a current workload checks
exporter association without filtering away disappeared-device history. A
**verified** association requires exact pod UID, namespace, container, node and
NVIDIA GPU/MIG UUID backed by the current DRA claim/slice mapping and native
cluster provenance. It does not prove historical allocation or exclusive physical
usage. Legacy device-plugin/name-only, ambiguous, missing or unsupported mappings
remain unverified or device-only; shared allocations and shared resource requests
remain device-only. A single-cluster acknowledgement is a declared unverified
assumption. Dedicated NVIDIA, MIG and sharing hardware compatibility remains
unverified until a real pilot is recorded.

## Explaining waiting GPU work

A pod investigation distinguishes admission, scheduling and startup with
observed, inferred or unknown confidence, capture time and source access/
completeness. Scheduler messages retain simultaneous constraints; startup uses
container and init-container state. DRA, PVC and configuration evidence remains
qualified rather than reproducing scheduler decisions. Optional Kueue
v1beta1/v1beta2 workload membership verifies referenced UIDs through at most four
controller hops. Quota reservation and admission checks do not guarantee pod
placement; no queue ETA/position or scheduling simulation is offered. Actual
Kueue controller compatibility remains unverified.
