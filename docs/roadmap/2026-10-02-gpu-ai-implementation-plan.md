# GPU operator workflows implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Explain waiting GPU workloads, expose their allocation, and optionally show historical usage.

**Architecture:** Three independently verifiable increments reuse Lumen's Tauri Kubernetes client, triage investigation, and device-resource page. Kubernetes readers produce bounded, sanitized evidence; telemetry uses an explicitly configured existing Prometheus Service through Kubernetes service proxy.

**Tech Stack:** React 19, TypeScript, TanStack Query, Tauri 2, Rust, kube 4.2, k8s-openapi 0.28 with `v1_35`, Vitest, wiremock, kind.

**Spec:** [Approved design](2026-10-02-gpu-ai-workflows-design.md).

## Global constraints

- Commands require an explicit non-empty context.
- Bound owner traversal to four hops, verify each referenced UID, and detect cycles.
- Kubernetes readers use 500-item pages, a 10,000-item per-source cap, and 15-second source deadlines, with client setup bounded separately.
- Offer 1 hour, 6 hours, 24 hours, and 7 days, with at most 1,000 returned points per series, 200 series, 4 MiB response size, and a 15-second request deadline.
- Preserve existing stable DRA behavior; do not widen its sanitized pod projection.
- No exporter installation, GPU mutation, automatic remediation, or remote publication.
- Keep the current Kubernetes API target and use existing dependencies where possible.

## Review focus

1. A pod name reused during collection must invalidate the investigation; pinned in scheduling task A1.
2. A malformed resource quantity or unsupported request form must produce unknown accounting; pinned in allocation task B1.
3. Owner cycles and namespace-restricted access must not imply membership or complete capacity; pinned in A2 and B2.
4. Missing telemetry identity and shared GPU series must not create exclusive per-pod usage; pinned in C2.
5. Source/context switches and oversized responses must not expose stale data or exhaust memory; pinned in A3, C1, and C3.

## Execution order

1. [A: Scheduling explanation](2026-10-02-gpu-scheduling-plan.md): A1 → A2 → A3.
2. [B: Allocation visibility](2026-10-02-gpu-allocation-plan.md): B1 → B2 → B3.
3. [C: Optional telemetry](2026-10-02-gpu-telemetry-plan.md): C1 → C2 → C3.

Each task owns a test cycle and a local commit. Keep the code on a dedicated
managed worktree at execution time, following using-git-worktrees. Preserve this
design branch and any unrelated checkout changes. A and B can ship independently
of C; do not call the full approved feature set complete until all three pass.

## Shared evidence contract

Create `src-tauri/src/k8s/gpu_types.rs` and `src/lib/gpuTypes.ts` in A1.
Rust structs serialize snake_case keys matching these TypeScript types:

```ts
export type SourceState = "available" | "unsupported" | "forbidden" | "error";
export type Source<T> = {
  state: SourceState; complete: boolean; captured_at: string;
  items: T[]; message: string | null;
};
export type ResourceRef = {
  api_version: string; kind: string; namespace: string | null;
  name: string; uid: string;
};
export type Explanation = {
  stage: "admission" | "scheduling" | "startup";
  confidence: "observed" | "inferred" | "unknown";
  message: string; sources: ResourceRef[]; captured_at: string;
  transition_time: string | null; observed_generation: number | null;
};
```

Unknown APIs and forbidden reads do not erase successful independent sources.
Use fixed error messages natively, never raw HTTP bodies or credential-plugin
stderr. Redact event/controller messages before IPC and before report rendering.
Unexpected resource replacement is an explicit invalid-target result, not an
empty successful snapshot.

## Completion checks for every increment

- [ ] Run targeted tests first, then the full checks once the increment is stable:

```sh
npm run lint
npm run test
npm run build
npm run perf:bundle
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm audit --audit-level=high
cargo audit --file src-tauri/Cargo.lock
./scripts/test-kind.sh
git diff --check
```

- [ ] Inspect keyboard access, narrow panels, empty/loading/partial/error states,
  and scoped links using the existing component-test patterns. Document blocked
  checks explicitly; unavailable kind/Docker or GPU hardware is not a passing test.
- [ ] Update `docs/FEATURES.md`, `docs/DEVICE_RESOURCES.md`,
  `docs/OPERATOR_DIAGNOSTICS.md`, and `docs/TEST_STRATEGY.md` for delivered behavior.
- [ ] Complete a branch review covering UID matching, namespace isolation,
  quantities, attribution, bounds, and secret-safe errors.
- [ ] Record real NVIDIA pilot evidence separately: dedicated GPU, MIG, and
  sharing. Until tested, describe hardware compatibility as unverified. Observe
  waiting-job explanation, allocation identification, and low-throughput triage.
- [ ] Report what shipped, which checks passed, remaining integration limitations,
  and local commits. Remote push/release needs separate user authorization.
