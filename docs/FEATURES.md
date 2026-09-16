# Lumen Features

Lumen is a local-first Kubernetes desktop workbench. It is built for operators
and developers who want a capable dashboard without routing cluster access
through a hosted control plane.

## Cluster Fleet

- Discover kubeconfig contexts and switch between clusters.
- Show cluster health, Kubernetes version, node readiness, and soft disconnects.
- Mark production-like contexts clearly so risky actions have stronger context.
- Inspect kubeconfig sources, definition precedence, and credential-tool availability from
  connection diagnostics, with separate guidance for authentication, TLS,
  network, and permission failures. Inspection does not execute credential tools.
- Merge all `KUBECONFIG` sources using Kubernetes first-definition-wins rules,
  including Windows path-list separators and source-relative credential paths.
  Context deletion and restoration retain source ownership.

## Workload Explorer

- Browse namespaces, workloads, services, storage, RBAC, policy, and custom
  resources from one desktop UI.
- Filter resources by name, namespace, kind, labels, status, images, restart
  count, and ownership.
- Open rich detail drawers with metadata, labels, owner references, events,
  YAML, pod state, metrics, and kind-specific insights.
- Remember namespaces per context, honor kubeconfig defaults, and enter a known
  namespace when discovery is denied. Workloads and triage retain successful
  results while explaining unavailable sources.
- Inspect custom resources by served version and namespace with CRD printer
  columns, schemas, and full controller conditions, including freshness.
- Create, edit, and delete custom-resource instances with RBAC checks, server
  validation, typed confirmation, and native context protection. Updates and
  deletion retain UID/revision preconditions; CRD definitions remain read-only.

See [custom resources and Helm repositories](CUSTOM_RESOURCES.md) for supported
printer paths, permissions, mutation behavior, and repository setup.

## Device Resources

- Browse ResourceClaims, ResourceClaimTemplates, DeviceClasses, and ResourceSlices
  through the stable Kubernetes DRA API, without installing an in-cluster agent.
- Follow pod claim references to allocations and their published device slices;
  inspect node inventory and driver-reported container health.
- Distinguish unsupported APIs, denied access, failed reads, and empty inventories.
  Missing health and unresolved node selectors remain unknown; inventory is not
  a measurement of free capacity.
- Inspect sanitized resource data in a read-only view. Opaque configuration,
  arbitrary driver data, and annotations are redacted.

See [device resources](DEVICE_RESOURCES.md) for API requirements and observation limits.

## Logs, Shell, And Port Forwarding

- Stream logs for pods and workload-owned pods with search, pause/resume, and
  bounded buffers.
- Open pod shell sessions through Kubernetes attach.
- Create unprivileged ephemeral debug containers for workloads without a shell,
  with configurable image, command, and filesystem profile. Reopen running
  diagnostic containers through the existing shell dock. Native protection and
  pod identity checks apply to creation and terminal access.
- Start and stop port forwards for pods and services from the local machine.

## Network Diagnosis

- Evaluate source egress and destination ingress together across selected
  namespaces, including per-destination named ports and numeric port ranges.
- Inspect Gateway listener, HTTPRoute parent, ReferenceGrant, Service, and
  endpoint relationships with the supporting conditions and resource names.
- Retain successful reads when other sources are denied. Missing data, stale
  status, unsupported selectors, controller identity, and unloaded namespaces
  remain explicit uncertainties. Configuration analysis does not prove traffic
  reachability and does not run active probes.

## Change And Incident Workflows

- Inspect rollout timelines, recent changes, events, alerts, and resource
  diffs.
- Generate incident reports with sensitive values redacted.
- Compare resources across clusters when troubleshooting drift.
- Investigate a selected triage issue with its failing container's current or
  retained previous logs, related events, and owner/controller snapshots. Export
  the selected evidence with capture times and unavailable-source notes.
- Match investigation events to resource UIDs; exclude name-only or mismatched
  events from verified evidence. Explicitly capture a bounded current or retained
  previous log excerpt, review and edit its redacted preview, and choose whether
  to include it in the report.
- Investigation snapshots do not establish historical rollout causality or
  recover discarded logs. Capture identifies the pod and requested container;
  unavailable evidence and limits on exact container-instance identity remain
  visible.

## Kubernetes Operations

- Protect selected contexts independently, with explicit ten-minute unlocks,
  persistent per-pane status, and native enforcement for cluster changes and
  pod shells. Protection survives restart; unlocks do not.
- Apply YAML only after successful server-side dry-run of the same draft,
  PATCH permission preflight, and confirmation. Both YAML entry points share
  these gates and respect the global read-only setting.
- Review original and draft YAML side by side, with added and removed lines,
  and explicitly revert unsubmitted edits. A refreshed resource preserves the
  draft and requires loading the latest version before applying. Reverting a
  draft does not undo a previously applied cluster change.
- Delete resources with confirmation and RBAC preflight checks.
- Manage Helm releases, including install, upgrade, rollback, and uninstall.
- Add, list, remove, and refresh local Helm HTTP(S) repositories from the chart
  picker, then choose a chart/version and deploy through the install wizard.
- Browse Argo CD applications, resource trees, history, and sync status.
- Inspect Tekton pipelines, pipeline runs, task runs, and status details.

See [protected contexts and kubeconfig sources](OPERATOR_SAFETY.md) for operation
and configuration details.
See [operator diagnostics](OPERATOR_DIAGNOSTICS.md) for diagnosis, debug-container,
and incident-handoff workflows and their limitations.

## Security Defaults

- Lumen reads the user's kubeconfig and talks directly to Kubernetes APIs.
- There is no hosted backend requirement.
- Secret YAML is redacted by default before rendering.
- Incident report exports redact obvious tokens, credentials, kubeconfig
  material, and secret values before report construction.
