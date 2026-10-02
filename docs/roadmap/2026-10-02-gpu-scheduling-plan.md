# GPU scheduling explanation implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Explain admission, scheduling, and startup blockers for a selected GPU pod.

**Architecture:** Native readers collect independently available Kubernetes evidence and optional Kueue relationships. A small panel adds that evidence to the existing triage investigation and export.

**Tech Stack:** Rust/kube, React/TypeScript, TanStack Query, Vitest, wiremock, kind.

**Spec:** [Approved design](2026-10-02-gpu-ai-workflows-design.md).

## Global constraints

All [shared constraints and completion checks](2026-10-02-gpu-ai-implementation-plan.md) apply.
Use explicit context and expected UID. Bound owner traversal to four hops.
No scheduler simulation, queue ETA, or cluster mutations.

## Review focus

Pod-name reuse; multiple simultaneous scheduler failures; owner cycles; stale
controller conditions; namespace-only RBAC. Tests below own these cases.

### A1: Selected-pod scheduling evidence

**Files:** Create `src-tauri/src/k8s/gpu_types.rs`, `gpu_scheduling.rs`,
`src-tauri/src/commands/gpu.rs`, `src/lib/gpuTypes.ts`, `gpuScheduling.ts`;
modify native module/command registration in `k8s/mod.rs`, `commands/mod.rs`,
and `lib.rs`. Add native tests inline in `gpu_scheduling.rs`.

**Interfaces:** `gpu_scheduling::snapshot(client: &Client, namespace: &str,
pod: &str, expected_uid: &str) -> AppResult<SchedulingSnapshot>`;
`gpu_scheduling::explain_pod(pod: &serde_json::Value,
events: &[serde_json::Value], captured_at: &str) -> Vec<Explanation>`.
`SchedulingSnapshot` contains `pod: ResourceRef`, `sources:
HashMap<String, Source<serde_json::Value>>`, and `explanations: Vec<Explanation>`.
IPC `gpu_scheduling_snapshot(context, namespace, pod, expected_uid)` maps to
`fetchGpuScheduling(context, namespace, pod, expectedUid): Promise<SchedulingSnapshot>`.

- [ ] Add native tests before implementation, including this exact event-identity case:

```rust
#[test]
fn excludes_an_event_for_a_replaced_pod() {
    let pod = serde_json::json!({"metadata":{"name":"train","uid":"new"},
      "status":{"phase":"Pending"}});
    let event = serde_json::json!({"involvedObject":{"uid":"old"},
      "reason":"FailedScheduling","message":"Insufficient nvidia.com/gpu"});
    let evidence = explain_pod(&pod, &[event], "2026-10-02T00:00:00Z");
    assert!(!evidence.iter().any(|e| e.message.contains("Insufficient")));
}
```

Add wiremock cases for UID replacement at the final pod read, event list denial,
node list denial, paginated/truncated collections, 15-second source timeout,
pending PVCs, unresolved claims, multiple scheduler reasons, and scheduled pods
whose init containers are waiting. Inject deadlines into internal helpers to
test timeouts cheaply. Verify env/Secret values never enter serialized output.

- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml gpu_scheduling` and
  confirm failure originates from the missing reader/explainer.
- [ ] Implement DTOs and explicit-context command validation using
  `commands/devices.rs` as the client-setup pattern. Collect pod, UID-filtered
  events, referenced PVCs/claims, and sanitized nodes concurrently with independent
  state/completeness. Re-read the pod at completion to reject UID replacement.
  Implement event selection with this identity rule:

```rust
let uid = pod.pointer("/metadata/uid").and_then(serde_json::Value::as_str);
let matched = events.iter().filter(|event| {
    uid.is_some() && event.pointer("/involvedObject/uid")
      .and_then(serde_json::Value::as_str) == uid
});
```

Classify scheduler statements as observed, matching configuration clues as
inferred, and unavailable evidence as unknown. Include source references,
timestamps, condition freshness, and redacted messages. Preserve whole original
scheduler messages; missing data cannot establish a successful placement.

- [ ] Run targeted native tests and `npm run lint`; inspect serialized contract
  parity and commit only A1 files with `feat: collect GPU scheduling evidence`.

### A2: Optional Kueue admission explanation

**Files:** Create `src-tauri/src/k8s/kueue.rs`, modify `gpu_scheduling.rs` and
`k8s/mod.rs`; create `src-tauri/tests/gpu_cluster.rs`.

**Interfaces:** `kueue::admission(client: &Client, namespace: &str,
pod: &serde_json::Value, captured_at: &str) -> Source<Explanation>`;
`kueue::matching_workload<'a>(workloads: &'a [serde_json::Value],
owners: &[ResourceRef]) -> Option<&'a serde_json::Value>`.
Add a `kueue` evidence source to the A1 snapshot. No frontend transport changes.

- [ ] Add this failing ownership test and wiremock tests for supported versions,
  quota reservation, admission checks, denied discovery, stale observedGeneration,
  same-name/wrong-UID Jobs, ambiguous matches, four-hop limit, and cycles:

```rust
#[test]
fn workload_name_does_not_establish_membership() {
    let workloads = vec![serde_json::json!({"metadata":{"name":"train",
      "ownerReferences":[{"kind":"Job","name":"train","uid":"other"}]}})];
    let owners = vec![ResourceRef { api_version:"batch/v1".into(),
      kind:"Job".into(), namespace:Some("team".into()), name:"train".into(),
      uid:"selected".into() }];
    assert!(matching_workload(&workloads, &owners).is_none());
}
```

- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml kueue` and confirm red.
- [ ] Implement version discovery, prefer known served v1beta2 then v1beta1,
  parse only supported condition/queue schemas, and retain API absence as
  unsupported. Verify every controller ancestor's UID before extending the chain;
  only controller owner references count. Match Workloads by namespace and owner
  UID; ambiguity yields unknown. Fetch only directly referenced queue/flavor/check
  objects. Preserve ordinary scheduling results after a Kueue failure.

The exact matching operation is:

```rust
let matches: Vec<_> = workloads.iter().filter(|workload| {
    workload.pointer("/metadata/ownerReferences").and_then(|v| v.as_array())
      .is_some_and(|refs| refs.iter().any(|r| owners.iter().any(|o| {
        r.get("uid").and_then(|v| v.as_str()) == Some(o.uid.as_str())
      })))
}).collect();
if matches.len() == 1 { Some(matches[0]) } else { None }
```

- [ ] Add kind integration cases in `gpu_cluster.rs`: scoped pod/event/PVC
  access, namespace isolation, schema-valid synthetic Kueue resources, and absent
  node permission. Reuse the isolated kubeconfig/cleanup convention of
  `devices_cluster.rs`; register `--test gpu_cluster` in `scripts/test-kind.sh`.
- [ ] Run targeted native and kind tests, then commit with
  `feat: explain Kueue admission for GPU workloads`.

### A3: Scheduling panel and incident evidence

**Files:** Create `src/components/gpu/GpuSchedulingPanel.tsx` and its test;
modify `src/components/TriageInvestigation.tsx`, its test, and
`src/lib/incidentReport.ts` only where selected observation formatting requires it.

**Interfaces:** `GpuSchedulingPanel({ context, namespace, pod, expectedUid,
onEvidence }: {context:string; namespace:string; pod:string; expectedUid:string;
onEvidence:(items:Explanation[])=>void})`. Consume A1's `fetchGpuScheduling`;
report only successfully refreshed selected-identity explanations.

- [ ] Add IPC-mocked component tests using the existing QueryClient/MemoryRouter
  wrappers. Verify separate stage labels, observed/inferred/unknown text, capture
  time, denied sources, unchanged pod names with new UIDs, and late results after
  context changes. Add an export test that includes current selected evidence and
  excludes cached explanations after a failed refresh.

Use this mock contract in the tests:

```ts
vi.mock("@/lib/gpuScheduling", () => ({ fetchGpuScheduling: vi.fn() }));
vi.mocked(fetchGpuScheduling).mockResolvedValue({
  pod: {api_version:"v1", kind:"Pod", namespace:"team", name:"train", uid:"u1"},
  sources: {}, explanations: [{stage:"scheduling", confidence:"observed",
    message:"Insufficient nvidia.com/gpu", sources:[],
    captured_at:"2026-10-02T00:00:00Z", transition_time:null, observed_generation:null}]
});
```

- [ ] Run `npx vitest run src/components/gpu/GpuSchedulingPanel.test.tsx
  src/components/TriageInvestigation.test.tsx` and confirm red.
- [ ] Implement the panel using existing SectionPanel/source-notice styles.
  Query key is `["gpu-scheduling", context, namespace, pod, expectedUid]`;
  `retry:false`, `refetchOnWindowFocus:false`, no background polling. Enable only
  when UID and namespace are available. Render `query.isError` before considering
  cached data. Key selected-issue state by UID and clear report observations on
  target changes, request failures, or pending replacement checks.
- [ ] Run targeted tests and the shared completion checks. Document supported
  explanations and capability limits; commit with `feat: investigate GPU scheduling`.
