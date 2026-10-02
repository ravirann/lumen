# GPU and AI workflows execution handoff

Implemented locally on `codex/gpu-ai-workflows` in the managed GPU worktree. The approved scope is delivered: waiting-workload scheduling/admission/startup evidence, GPU requests and advertised node slots alongside DRA allocation, and optional historical GPU telemetry through a configured Kubernetes Service.

## Verification

Final implementation revision: `16f0eae1ef3cf3fc433f7e309747f8325075578b`.

| Check | Result |
| --- | --- |
| Frontend suite | 683 tests / 93 files passed |
| Native suite | 271 unit tests and fleet integration passed |
| Type checking / production build / bundle budgets | Passed |
| Rust formatting / all-target Clippy / diff whitespace | Passed |
| Isolated kind | Six integration tests across five binaries passed; disposable clusters cleaned up |
| npm audit, high threshold | Passed; two existing moderate Vitest advisories unchanged |
| cargo audit | Passed; seven existing allowed warnings unchanged |

Kind used synthetic Kubernetes/Kueue status and Prometheus JSON, with temporary fixture kubeconfig and actual Service-proxy RBAC. Final fixes changed no transport or cluster fixtures. Full frontend/native/build checks were repeated after those fixes. React act/Vite warnings remain baseline noise. An intermediate parallel storage test failure did not recur in the final standard native runs. Intermediate frontend failures captured deliberate concurrent regression RED phases; the final suite has no failures.

## Review outcome

Each of the nine tasks received an independent spec and quality review. The whole-branch review then identified five important integration issues and two minor issues. One consolidated fix commit resolved native spaced-secret redaction, DRA-only workload selection, unreconciled instance qualifiers, absent/trailing telemetry evaluations, aggregate capability deadlines, claim filtering links, and known shared-allocation labels. The strongest-model scoped re-review accepted every fix with no remaining load-bearing finding.

## Pilot limits

Dedicated NVIDIA, MIG, shared GPU hardware, actual DCGM exporter and real Kueue-controller behavior remain unverified. Current DRA-backed exporter association does not prove historical allocation or exclusive physical usage. Instance-qualified series remain unverified until an explicit supported mapping can reconcile them. Missing metrics/samples remain unavailable rather than idle. No exporter installation, GPU mutation, remediation, source push or release was performed.

## Rulings and rework costs

1. Parallel independent task work is authorized by the user's explicit request despite the subagent skill's default serial implementation rule — use file ownership and serialized shared registration/commits — mistakes would cost integration rework.
2. Separate commands/gpu_telemetry.rs and gpuTelemetryTypes.ts for telemetry rather than editing scheduling command/type files concurrently — preserve IPC contract while isolating files — mistakes would cost small registration/type consolidation rework.
3. task-brief helper only recognizes numbered Task headings; extract existing A1/B1/C1 headings directly without modifying approved plans — retain verbatim requirements — mistakes would cost redispatch if extraction were incomplete.
4. Add direct http-body=1 dependency already present transitively in Cargo.lock for streaming decoded response limits — avoids unbounded error-body parsing — cost if wrong is a small dependency/transport revision.
5. Preserve existing audit warnings and act/configLoader warning baselines rather than fold unrelated dependency/test changes into GPU feature work — focus authorized scope — cost if wrong is separate maintenance work.
6. Put inventory snapshot type in its owning inventory module and import shared DTOs — avoids concurrent shared type edits without changing external contract — cost if wrong is small import refactor.
7. Isolate A2 kind tests in kueue_cluster.rs instead of B2 gpu_cluster.rs — prevents concurrent fixture edits and keeps controller-specific validation focused — cost if wrong is an extra test-binary registration.
8. Use explicit NVIDIA DRA UUID mapping as bounded verified telemetry path, with exact pod UID/container and native cluster provenance; unsupported mappings remain unverified — meets approved evidence requirement — cost if wrong is adapter correction, not inferred physical allocation.
9. Node links use existing DRA device detail route with explicit DRA label and retained context/namespace — NodesView does not consume node selection — cost if wrong is additional node routing UX work.
10. Add optional onSources callback alongside required onEvidence to export access/completeness/capture time without fabricating Explanation records — preserve evidence categories — cost if wrong is removing one optional prop and guarded local state.
11. Retain triage resource-ref type and gate scheduling/export by metadata-observed UID plus context/namespace/name; refreshing or failed metadata excludes prior observations and UID replacement remounts panel — avoid widening upstream issue contract — cost if wrong is extending selected identity/upstream issue construction later.
12. Selected workload stays a local attribution input rather than native history query/cache input — history API intentionally independent of live inventory — cost if wrong is future identity-filtered API/key expansion.
13. Kind asserts computed native cluster-provenance marker instead of arbitrary cluster label echo — preserve sanitized metric label allowlist — cost if wrong is leaving raw-selector troubleshooting in setup.
14. Root owns stable combined completion checks after parallel UI commits — concurrent red windows are not final suite failures — cost if wrong is targeted checks failing to establish aggregate readiness.
15. Instance-qualified exporter series remain unverified until their qualifiers can be reconciled with an exact supported device mapping — fail closed against parent GPU confusion — cost if wrong is a later exporter-specific identity adapter.

## Local implementation commits

```text
f8ec75d feat: account for GPU workload requests
a43e2e2 feat: collect GPU scheduling evidence
cb7f447 feat: configure bounded GPU telemetry
b1e3873 feat: collect scoped GPU inventory
dfc0175 fix: redact quoted GPU evidence credentials
985f170 feat: query and attribute GPU usage history
ad9f233 feat: show GPU allocation alongside device resources
dfa7fd2 fix: verify GPU associations and bound history deadline
596414b fix: keep unsafe GPU quantities unknown in inventory rows
13e3a29 feat: explain Kueue admission for GPU workloads
460db71 fix: preserve Kueue ancestor read failure states
a50d3fc feat: show optional historical GPU telemetry
09cf684 feat: investigate GPU scheduling
16f0eae Fix GPU evidence redaction and telemetry association safeguards
```

The original design checkout is preserved. Keep the implementation worktree for review or later integration; remote publication requires a separate instruction.
