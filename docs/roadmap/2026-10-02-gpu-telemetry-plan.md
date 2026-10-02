# Optional GPU telemetry implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show bounded historical GPU usage from a configured existing Prometheus Service.

**Architecture:** Native per-context configuration and Kubernetes service-proxy transport feed fixed query families. Native parsing sanitizes metrics; frontend attribution and charts preserve missing identity and sharing limitations.

**Tech Stack:** Rust/kube/http/futures, TypeScript/React, existing SVG components, Vitest, wiremock, kind.

**Spec:** [Approved design](2026-10-02-gpu-ai-workflows-design.md).

## Global constraints

All [shared constraints and completion checks](2026-10-02-gpu-ai-implementation-plan.md) apply.
No separate telemetry credentials, arbitrary URL, PromQL editor, or TLS bypass.
At most 1,000 points per series, 200 series, 4 MiB, and 15 seconds per request.
Allowed windows: 1 hour, 6 hours, 24 hours, 7 days. No background scrape loop.

## Review focus

Literal-label injection; compressed oversized bodies; ambiguous pod/device
identity; shared metrics counted repeatedly; old-source results after switching.

### C1: Configuration and bounded service-proxy transport

**Files:** Create `src-tauri/src/gpu_settings.rs`,
`src-tauri/src/k8s/gpu_telemetry.rs`; modify `state.rs`, native registration, and
`commands/gpu.rs`; create `src/lib/gpuTelemetry.ts` and shared DTOs.

**Interfaces:** `GpuTelemetryConfig { namespace:String, service:String,
port:String, cluster_label:Option<String>, cluster_value:Option<String>,
single_cluster_acknowledged:bool }`; `GpuSettings::load(path:PathBuf)`,
`get(context:&str)->Option<GpuTelemetryConfig>`,
`set(context:&str, config:Option<GpuTelemetryConfig>)->AppResult<()>`.
IPC `gpu_telemetry_config_get`, `gpu_telemetry_config_set`, and
`gpu_telemetry_capabilities`, each with explicit context.
`validate_config(&GpuTelemetryConfig)->AppResult<()>` and
`proxy_get(client:&Client, config:&GpuTelemetryConfig,
suffix:&str)->AppResult<Vec<u8>>` are internal transport interfaces.

- [ ] Write failing tests for Service/namespace DNS validation, named/numeric
  ports, exact cluster-label literals, missing acknowledgement, config load/write
  failures, context isolation, RBAC preflight, 403/404 fixed guidance, response
  bounds after decoding, timeout, and no response-body errors leaked to IPC.

```rust
#[test]
fn rejects_arbitrary_service_paths() {
    let c = GpuTelemetryConfig { namespace:"monitoring".into(),
      service:"prometheus/../../secrets".into(), port:"9090".into(),
      cluster_label:None, cluster_value:None, single_cluster_acknowledged:true };
    assert!(validate_config(&c).is_err());
}
```

- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml gpu_telemetry` and
  `cargo test --manifest-path src-tauri/Cargo.toml gpu_settings`; confirm red.
- [ ] Implement native config with atomic temporary-file replacement and
  synchronization; load from `AppState::with_config_dir`, not frontend storage.
  Failed persistence is an error, not a successful toast. Use the Kubernetes
  service proxy path `/api/v1/namespaces/{namespace}/services/{service}:{port}/proxy`.
  Permit only internally built `/api/v1/query` and `/api/v1/query_range` suffixes.
  Preflight get on `services/proxy`; actual API permission remains authoritative.
  Encode query parameters using a focused RFC3986 byte encoder with regression
  tests for `%`, quotes, Unicode, `+`, and `&`; no new URL dependency required.
  Bound the decoded response while streaming rather than reading an unbounded
  JSON body. Verify kube's installed response-stream/decompression API locally
  before choosing its implementation; preserve 4 MiB and deadline limits even
  if an explicit gzip decoder is needed.

The bounded-byte accumulator contract is:

```rust
fn append_bounded(output: &mut Vec<u8>, next: &[u8]) -> AppResult<()> {
    if output.len().checked_add(next.len()).is_none_or(|n| n > 4 * 1024 * 1024) {
        return Err(AppError::K8s("Telemetry response exceeded 4 MiB; narrow the query.".into()));
    }
    output.extend_from_slice(next);
    Ok(())
}
```

- [ ] Add the new config state without changing context protection/attach/forward
  behavior; run targeted tests and commit with `feat: configure bounded GPU telemetry`.

### C2: Fixed metrics, historical parsing, and attribution

**Files:** Modify `gpu_telemetry.rs`, `gpu_types.rs`, `commands/gpu.rs`,
`gpuTelemetry.ts`, `gpuTypes.ts`; create `src/lib/gpuTelemetry.test.ts`.

**Interfaces:** `GpuMetricFamily` enum: `Utilization`, `FramebufferUsed`,
`FramebufferTotal`, `SmActive`, `TensorActive`, `XidErrors`.
Fixed initial names: `DCGM_FI_DEV_GPU_UTIL`, `DCGM_FI_DEV_FB_USED`,
`DCGM_FI_DEV_FB_TOTAL`, `DCGM_FI_PROF_SM_ACTIVE`,
`DCGM_FI_PROF_PIPE_TENSOR_ACTIVE`, `DCGM_FI_DEV_XID_ERRORS`.
Capability output lists available families/identity-label coverage, not a claimed
exporter version inferred from metric names.
`GpuSeries { family:GpuMetricFamily, labels:HashMap<String,String>,
points:Vec<(f64,Option<f64>)> }`;
`GpuHistory { captured_at:String, complete:bool, warnings:Vec<String>,
series:Vec<GpuSeries> }`.
Serialize metric families as snake_case names matching the TypeScript union
`"utilization" | "framebuffer_used" | "framebuffer_total" | "sm_active" |
"tensor_active" | "xid_errors"`.
IPC `gpu_telemetry_history(context, namespace, window_seconds, end_seconds)`;
`fetchGpuHistory(context,namespace,windowSeconds,endSeconds):Promise<GpuHistory>`;
`attributeGpuSeries(series:GpuSeries, pod:GpuPod,
shared:boolean):"verified"|"unverified"|"device-only"`.

- [ ] Add failing native tests for literal PromQL label escaping, exact allowed
  metric names, valid windows, 200-series/1,000-point limits, NaN/Inf as gaps,
  duplicate or out-of-order timestamps, unknown labels excluded, unsupported
  families, absent cluster provenance, and device history after node deletion.
  Reject nonfinite, negative, and future end timestamps; reject any start/end
  pair whose duration differs from an allowed window. Use an injected clock
  for deterministic validation, with no caller-controlled clock bypass in IPC.
  Add this frontend attribution test:

```ts
it("does not assign shared-device activity exclusively to a pod", () => {
  const pod = {resource:{api_version:"v1",kind:"Pod",namespace:"team",
    name:"train",uid:"u1"},phase:"Running",node_name:"gpu",owners:[],requests:{}};
  const series = {family:"utilization" as const,labels:{UUID:"GPU-1",
    namespace:"team",pod:"train",pod_uid:"u1"},points:[]};
  expect(attributeGpuSeries(series,pod,true)).toBe("device-only");
});
```

- [ ] Run targeted native tests and `npx vitest run src/lib/gpuTelemetry.test.ts`;
  confirm red.
- [ ] Implement bounded instant capability queries and range queries with fixed
  family names. Step is `ceil(window_seconds / 999)` so the inclusive endpoints
  yield no more than 1,000 points; enforce limits again on returned data. Escape
  exact label values with JSON string encoding before URL encoding. Source
  configuration controls cluster filtering; namespace queries do not pretend to
  add Prometheus authorization. Bound device-level queries separately from
  namespace workload series to avoid discarding unallocated device observations.
  Use the selected endpoint as declared provenance when no cluster label exists,
  visibly marked unverified. Verify workload attribution only with selected UID,
  namespace, device/container identity, and unambiguous cluster mapping. Shared or
  mismatched series stay device-only; name-only associations stay unverified.
- [ ] Parse units explicitly: utilization percentage versus profiling fractions,
  DCGM framebuffer MiB versus bytes. Treat absent/nonfinite samples as gaps,
  never zero utilization. XID readings are error observations, not automatically
  error counts or root causes. Retain disappeared-device series without joining
  against only currently live nodes.
- [ ] Run targeted tests and commit with `feat: query and attribute GPU usage history`.

### C3: Optional setup, usage charts, and integration coverage

**Files:** Create `src/components/gpu/GpuTelemetrySetup.tsx`,
`GpuUsagePanel.tsx`, and their tests; modify device/inventory panels;
extend `gpu_cluster.rs`, add fixtures in `testdata/k8s/gpu-telemetry.yaml`, and
update feature/diagnostics/test documentation.

**Interfaces:** `GpuTelemetrySetup({context,onSaved}:{context:string;
onSaved:()=>void})`; `GpuUsagePanel({context,namespace}:{context:string;
namespace:string})`. Frontend wrappers consume C1 and C2's registered IPC.
No new chart library: use bounded SVG sparklines with text values and accessible
metric labels; downsample display without changing exported measurement data.

- [ ] Add IPC-mocked component tests for absent configuration, persistence
  failure, denied service proxy, missing families, explicit single-cluster
  acknowledgement, cluster-label configuration, ambiguous mappings, shared
  devices, gaps, oversized responses, and context/source changes in flight.
  Verify no polling after unmount and no initial requests before entering usage.

```ts
it("retains unsupported readings as gaps instead of idle GPUs", () => {
  expect(normalizeGpuPoint("NaN")).toBeNull();
  expect(normalizeGpuPoint("+Inf")).toBeNull();
  expect(normalizeGpuPoint("0")).toBe(0);
});
```

Define `normalizeGpuPoint(value:string):number|null` in `gpuTelemetry.ts`;
it mirrors native finite-value handling for chart inputs.

- [ ] Run `npx vitest run src/components/gpu/GpuTelemetrySetup.test.tsx
  src/components/gpu/GpuUsagePanel.test.tsx src/lib/gpuTelemetry.test.ts`; confirm red.
- [ ] Implement setup with only Service fields and cluster provenance, connection
  test, remove-source action, and explicit service-proxy data-access explanation.
  Use native persisted config; successful save invalidates capability/history
  keys. Include serialized config, context, namespace, window, and end time in
  query keys; `retry:false`, `refetchOnWindowFocus:false`, no `refetchInterval`.
  Keep stale data visibly historical or unavailable after failure; never relabel
  it as a fresh observation. Mount usage only when selected; display capability
  and identity coverage before graphs.
- [ ] Add a synthetic Prometheus fixture Service serving deterministic JSON and
  a scoped role with and without `services/proxy` get permission. Extend the
  isolated kind integration test to verify proxy access, denied access, literal
  selectors, and native byte/series limits. Use synthetic data only; document that
  it does not prove DCGM hardware mapping.
- [ ] Run targeted tests and all shared completion checks; record GPU pilot
  status and commit with `feat: show optional historical GPU telemetry`.
