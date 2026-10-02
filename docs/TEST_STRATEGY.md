# Test Strategy

Lumen must be tested as a production Kubernetes operations tool. The test suite should verify correctness, security, performance, and UX across realistic clusters, not just component rendering.

## Test Pyramid

1. Unit tests for parsing, resource registry behavior, diffing, RBAC gates, secret redaction, filters, and health calculations.
2. Rust tests for Kubernetes command handlers, API path construction, dry-run behavior, watch handling, port-forward lifecycle, and error mapping.
3. Component tests for resource lists, details, dialogs, command palette, search, YAML editor, logs, shell, and settings.
4. Integration tests against fixture Kubernetes API responses.
5. End-to-end tests against disposable `kind` and `k3d` clusters.
6. Performance and accessibility checks on large synthetic clusters.

## Required CI Gates

- `npm run lint`
- `npm run test`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
- `npm audit --audit-level=high`
- `cargo audit`
- Secret scan
- CodeQL or equivalent static analysis

## Kubernetes E2E Matrix

`./scripts/test-kind.sh` currently provides a native API integration gate for
server dry-run, apply/delete, read-only RBAC and namespace isolation. It creates
a disposable cluster with a temporary kubeconfig and deletes it on exit. It
does not exercise the desktop UI. The broader matrix below remains the release
validation target, rather than a claim that every scenario is automated.

Run the core E2E suite on:

- `kind` latest stable Kubernetes.
- `kind` previous supported minor.
- `k3d` latest stable.
- Metrics Server installed and absent.
- Restricted RBAC user.
- Read-only RBAC user.
- Namespace-scoped user.

## Headlamp Parity E2E Scenarios

- Add/select context and namespace.
- Search for resources by name, label, namespace, kind, and API group.
- Browse every v1.0 resource kind in the parity matrix.
- Open detail view for every kind and verify metadata, YAML, events, and related objects render.
- Create resource from pasted YAML.
- Create resource from file upload.
- Edit resource with server dry-run diff.
- Delete resource with confirmation.
- Scale Deployment/StatefulSet.
- Restart Deployment/StatefulSet/DaemonSet.
- Trigger CronJob.
- View logs for Pod and workload-owned Pods.
- Open Pod shell with container selector.
- Start and stop port-forward.
- View metrics with Metrics Server installed.
- See clear no-metrics notice without Metrics Server.
- Mask, reveal, copy, and re-mask Secret fields.
- Browse RBAC resources and verify forbidden actions are disabled for restricted users.

## Performance Budgets

- App shell initial usable state: under 2.5 seconds on a warm local build.
- Main application chunk: under 500 kB minified, with route/resource-family code splitting for parity screens.
- Context switch to first resource list: under 1.5 seconds after Kubernetes API responds.
- Resource list scroll: no visible jank with 10,000 rows.
- Events view: handles 20,000 events without locking the renderer.
- Logs: 50,000 lines per stream without unbounded memory growth.
- Global search: responsive under 100 ms for cached resources up to 50,000 objects.
- Cloud/resource map: progressive rendering for graphs over 1,000 nodes.

## UX and Accessibility Gates

- Keyboard navigation for command palette, dialogs, tables, tabs, and drawer actions.
- Visible focus states.
- No clipped text on 320 px mobile-width windows or narrow desktop side panels.
- Color contrast meets WCAG AA for primary text and controls.
- Destructive actions use consistent confirmation language.
- Production context warnings are visually distinct and not color-only.
- Empty, loading, partial-permission, and error states exist for every major view.

## Security Regression Tests

- Cluster-provided HTML in labels, annotations, events, and logs renders inert.
- Secret values do not appear in console logs, error toasts, activity records, or test snapshots.
- RBAC-denied users do not see enabled destructive controls.
- Dry-run failures block final apply.
- Conflicting `resourceVersion` update asks for reload.
- Kubeconfig exec plugin errors are reported without exposing tokens.
- Port-forward and shell sessions close on context change, app shutdown, and explicit stop.

## Operator Workflow Regression Coverage

- Namespace resolution: explicit URL, per-context selection, kubeconfig default,
  failed/missing default lookup, manual selection with denied discovery, and
  no broad requests or manual refresh while scope is unresolved.
- Partial workload and triage access: retain successful sources and identify
  unavailable sources; namespace/context changes discard old selections and
  streamed events.
- Connection diagnostics: secret-safe fixed guidance, executable availability,
  Windows path parsing, and pending/failed inspection and retry lifecycles.
- Investigation: selected-resource events, failing-container log links,
  owner/controller evidence, partial exports, and late results from old targets.
- Both YAML editors: PATCH preflight, exact-draft dry-run, draft invalidation,
  target changes, and enabling read-only during an in-flight dry-run.

Browser smoke checks can use synthetic Tauri IPC fixtures to inspect layouts
and route/container selection. These checks complement component tests; they
do not prove native credential execution or installed-app behavior on macOS,
Windows, or Linux.

## GPU allocation and optional telemetry coverage

GPU allocation frontend tests cover namespace scope and late responses,
denied/partial sources, unknown checked sums, unsafe quantities, terminal
exclusion, separate MIG/shared keys, absent DRA APIs, scoped DRA navigation,
refresh failure, keyboard tabs, pagination and narrow table scrolling.

Optional usage tests mock IPC for absent configuration, explicit single-cluster
acknowledgement and exact cluster-label configuration, persistence failure,
connection denial/removal, missing families and identity labels, historical
nonfinite gaps, disappeared inventory, shared attribution, bounded-response
failure, failed-refresh clearing and context/source switches in flight. Route
coverage checks no telemetry requests before Usage history selection; unmount
coverage checks no background polling. Pure attribution tests cover exact DRA
UUID/MIG mapping and conservative ambiguity/sharing handling. Native tests cover
fixed literal selectors, sanitization, timestamp/unit validation, aggregate
deadlines, decoded gzip bounds and response/series/point caps.

`scripts/test-kind.sh` creates a disposable local cluster with a temporary
KUBECONFIG and deletes it on exit. `testdata/k8s/gpu-telemetry.yaml` runs a
synthetic deterministic HTTP JSON Service, scoped `services/proxy` GET role and
denied identity. The `gpu_cluster` integration checks permission preflight and
actual proxy denial, cross-namespace denial, literal cluster selectors,
workload/device history separation, zero/nonfinite gap retention, private-label
redaction, 200-series and 4-MiB rejection. It does not contact user clusters or
install a real NVIDIA/DCGM exporter. Fixtures prove transport and bounded parsing,
not actual GPU allocation or exporter correctness. Real dedicated NVIDIA, MIG,
sharing and controller pilot compatibility remains unverified.

Scheduling investigation tests distinguish admission/scheduling/startup,
observed/inferred/unknown confidence and access/completeness limits. Export tests
exclude stale explanations after pending/failed refresh, context/scope changes
and UID replacement, and apply final report redaction. Native tests and isolated
Kueue CRD fixtures cover v1beta1/v1beta2 adapters, four-hop UID ownership, cycles
and directly referenced support objects. Synthetic CRDs prove reader behavior,
not real Kueue controller admission or GPU placement.
