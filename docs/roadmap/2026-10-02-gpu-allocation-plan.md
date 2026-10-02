# GPU allocation visibility implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show GPU workload requests, node advertised capacity, and existing DRA allocations without conflating slots with devices.

**Architecture:** A purpose-specific inventory command sanitizes nodes and pods. Reusable panels extend the existing device page without modifying its DRA projection.

**Tech Stack:** Rust/kube, TypeScript/React, Vitest, wiremock, kind.

**Spec:** [Approved design](2026-10-02-gpu-ai-workflows-design.md).

## Global constraints

All [shared constraints and completion checks](2026-10-02-gpu-ai-implementation-plan.md) apply.
No cluster-wide free-GPU calculation. Terminal pods do not count as active requests.
Support NVIDIA GPU, shared-GPU, and MIG extended-resource keys initially.

## Review focus

Restartable init requests; malformed quantities; namespace-limited totals;
device-plugin/DRA double counting; advertised shared slots mistaken for devices.

### B1: Effective accelerator request accounting

**Files:** Create `src-tauri/src/k8s/gpu_inventory.rs`, register in `k8s/mod.rs`;
add inline native tests. Extend `gpu_types.rs` and `src/lib/gpuTypes.ts`.

**Interfaces:** `effective_gpu_requests(pod: &serde_json::Value) ->
HashMap<String, Option<i64>>`; `is_gpu_resource(name:&str)->bool`;
`active_pod(pod:&serde_json::Value)->bool`.
New DTOs: `GpuPod { resource:ResourceRef, phase:String, node_name:Option<String>,
requests:HashMap<String,Option<i64>>, owners:Vec<ResourceRef> }` and
`GpuNode { resource:ResourceRef, allocatable:HashMap<String,Option<i64>>,
gpu_labels:HashMap<String,String> }`.

- [ ] Add these failing cases plus restartable-init, sequential-init, overhead,
  limits-defaulting, negative/fractional/overflow quantities, and unsupported
  request-form tests:

```rust
#[test]
fn invalid_quantity_remains_unknown() {
    let pod = serde_json::json!({"spec":{"containers":[{"name":"worker",
      "resources":{"requests":{"nvidia.com/gpu":"garbage"}}}]}});
    assert_eq!(effective_gpu_requests(&pod).get("nvidia.com/gpu"), Some(&None));
}
#[test]
fn completed_jobs_are_not_active_allocations() {
    assert!(!active_pod(&serde_json::json!({"status":{"phase":"Succeeded"}})));
    assert!(!active_pod(&serde_json::json!({"status":{"phase":"Failed"}})));
}
```

- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml gpu_inventory`; confirm red.
- [ ] Implement checked integer Kubernetes quantity conversion using its exact
  decimal/binary/exponent semantics; GPU quantities must resolve to whole
  non-negative integers. Reject malformed and overflowing input. Use known
  Kubernetes pod request semantics: sum application and restartable-init steady
  requests; evaluate each sequential init with preceding restartable-init requests;
  take the maximum stage per resource, then add applicable overhead. Any unknown
  contributing quantity keeps that resource unknown. Tests include limit-only
  admitted API objects and unsupported pod-level resource forms.

Resource-key recognition is exactly:

```rust
pub fn is_gpu_resource(name: &str) -> bool {
    matches!(name, "nvidia.com/gpu" | "nvidia.com/gpu.shared")
      || name.strip_prefix("nvidia.com/mig-").is_some_and(|s| !s.is_empty())
}
```

- [ ] Run targeted tests and commit with `feat: account for GPU workload requests`.

### B2: Bounded GPU inventory command

**Files:** Modify `gpu_inventory.rs`, `src-tauri/src/commands/gpu.rs`,
`src-tauri/src/lib.rs`; create `src/lib/gpuInventory.ts`; extend `gpu_cluster.rs`.

**Interfaces:** `gpu_inventory::snapshot(client:&Client, namespace:&str) ->
GpuInventorySnapshot { namespace:String, captured_at:String, pods:Source<GpuPod>,
nodes:Source<GpuNode> }`; IPC `gpu_inventory_snapshot(context, namespace)`;
`fetchGpuInventory(context:string, namespace:string):Promise<GpuInventorySnapshot>`.

- [ ] Add wiremock tests for multi-page pods/nodes, 10,000-object truncation,
  deadlines, node denial, empty available sources, and restricted namespaces.
  Test serialization directly:

```rust
#[test]
fn projection_excludes_environment_values() {
    let pod = serde_json::json!({"apiVersion":"v1","kind":"Pod",
      "metadata":{"name":"train","namespace":"team","uid":"u1"},
      "spec":{"containers":[{"name":"worker","env":[{"value":"private-marker"}],
      "resources":{"limits":{"nvidia.com/gpu":"1"}}}]}});
    let projected = serde_json::to_string(&project_gpu_pod(&pod)).unwrap();
    assert!(!projected.contains("private-marker"));
}
```

Define `project_gpu_pod(pod:&serde_json::Value)->GpuPod` in this task. Add
`project_gpu_node(node:&serde_json::Value)->GpuNode` with an allowlist for GPU
product, MIG, replicas, and sharing labels; arbitrary node labels stay outside IPC.

- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml gpu_inventory`; confirm red.
- [ ] Implement independent paginated node/pod collection, explicit namespace,
  client timeout, sanitized failures, and typed projections. Preserve incomplete
  source metadata after truncation. Do not export env, annotations, or full objects.
  Keep physical device mapping unknown for device-plugin requests.
- [ ] Extend kind tests with a namespace-only reader, GPU-request pod, terminal
  pod, namespace isolation, and serialization redaction; run targeted and kind tests.
- [ ] Commit with `feat: collect scoped GPU inventory`.

### B3: GPU workload and node views

**Files:** Create `src/components/gpu/GpuInventoryPanels.tsx` and test;
modify `DeviceResourcesView.tsx` and test; create `src/lib/gpuInventory.test.ts`.

**Interfaces:** `GpuInventoryPanels({context,namespace}:{context:string;
namespace:string})`; `summarizeGpuRequests(pods:GpuPod[]):Record<string,
number|null>` in `gpuInventory.ts`. Consume B2. Existing DRA links use
`src/lib/deviceResources.ts` without a changed command contract.

- [ ] Add a failing aggregate test:

```ts
it("keeps MIG and shared slots in separate request totals", () => {
  const base = {resource:{api_version:"v1",kind:"Pod",namespace:"team",
    name:"train",uid:"u1"},phase:"Running",node_name:"gpu-node",owners:[]};
  expect(summarizeGpuRequests([
    {...base, requests:{"nvidia.com/gpu.shared":8}},
    {...base, requests:{"nvidia.com/mig-1g.5gb":2}}
  ])).toEqual({"nvidia.com/gpu.shared":8,"nvidia.com/mig-1g.5gb":2});
});
```

Add component tests for node denial with usable workloads, partial totals,
unknown quantities, no DRA API, no GPU requests, unchanged existing DRA navigation,
late context responses, scoped links, and keyboard-accessible tabs.

- [ ] Run `npx vitest run src/lib/gpuInventory.test.ts
  src/components/gpu/GpuInventoryPanels.test.tsx
  src/routes/cluster/DeviceResourcesView.test.tsx`; confirm red.
- [ ] Implement per-resource checked sums with null propagation and terminal-pod
  exclusion. Label summaries “Observed namespace requests” or “Observed requests”;
  show completeness. Add workload/node tabs to the device page and reuse existing
  detail links. Node allocatable rows say “Advertised slots”; no physical/free GPU
  inference. Keep DRA allocations separate with driver/pool/device identifiers.
- [ ] Run targeted and shared completion checks; update delivered-feature docs
  and commit with `feat: show GPU allocation alongside device resources`.
