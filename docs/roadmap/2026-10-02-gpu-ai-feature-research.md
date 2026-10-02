# GPU and AI workload feature research

Research date: October 2, 2026. Local baseline: `4606411`.
Audience: teams running GPU and AI workloads, selected by the user.
This is a research recommendation, not an approved implementation specification.

## Recommendation

Build a local operator workflow that answers: why is my AI workload waiting,
which devices does it use, and is it doing useful work? Prioritize scheduling
explanations and GPU allocation/telemetry before broad ML platform integrations.

The ranking below weighs operator value, fit with Lumen, dependencies, and
maintenance burden. Sizes are relative scope, not delivery estimates. Public
documentation establishes workflow feasibility; it does not establish demand
among Lumen users. No interviews or competitor usability tests were conducted.
The live GitHub open-issue query returned no issues for `ravirann/lumen`.

## Existing foundations and actual gaps

- `docs/DEVICE_RESOURCES.md`, `DeviceResourcesView.tsx`, and `devices.rs` already
  cover stable DRA discovery, claims, published inventory, pod attribution, and
  driver-reported health. They do not measure utilization or cover a dedicated
  legacy device-plugin GPU workflow. Do not rebuild generic DRA inspection.
- `src/lib/triage.ts` detects Pending pods and gives general next steps. A
  correlated scheduling explanation is the new capability.
- `MetricsExplorerView.tsx`, `metrics_explorer.rs`, and
  `resourceRecommendations.ts` already cover CPU/memory snapshots and resource
  suggestions. Historical GPU telemetry is the gap; another generic metrics
  screen is unnecessary.
- Generic CRD editing, logs, workload relationships, protected contexts, debug
  containers, network configuration analysis, and incident exports exist.
  Dedicated Kueue, Ray, Kubeflow, KServe, DCGM, and vLLM adapters were not found
  in the inspected source.

## Ranked candidates

| Rank | Feature | First useful version | Relative scope |
| --- | --- | --- | --- |
| 1 | Explain waiting GPU jobs | Correlate scheduler events, GPU requests, placement constraints, PVCs, DRA claims, and optional Kueue admission status | M–L, staged |
| 2 | GPU allocation and usage | Show advertised resources and pod requests for device-plugin clusters alongside DRA; optionally join DCGM/Prometheus history | L, staged |
| 3 | AI workload lifecycle views | One initial adapter: RayJob/RayCluster/RayService linked to head/worker pods, conditions, logs, and GPUs | M per adapter |
| 4 | Inference performance | vLLM request queue, time to first token, token throughput, and cache metrics beside workload health | M–L after telemetry |
| 5 | Distributed workload investigation | One selected job's worker failures, logs, topology evidence, network checks, and incident export | L; narrow pilot first |

### 1. Explain waiting GPU jobs

Distinguish queue admission from pod scheduling and container startup. Surface
reported reasons with source objects and timestamps: insufficient requested
resources, incompatible GPU placement, pending claims, storage binding, or a
Kueue admission check. Explain that idle hardware does not imply schedulable
capacity. Avoid promising a scheduling simulation or queue start-time estimate.

Kubernetes documents [Pending pod diagnosis](https://kubernetes.io/docs/tasks/debug/debug-application/debug-running-pod/).
Kueue's [job troubleshooting](https://kueue.sigs.k8s.io/docs/tasks/troubleshooting/troubleshooting_jobs/)
and [pending-workload API](https://kueue.sigs.k8s.io/docs/tasks/manage/monitor_pending_workloads/)
provide concrete data sources. A [heterogeneous accelerator discussion](https://github.com/kubernetes-sigs/kueue/discussions/11090)
is an additional user signal, not a prevalence estimate.

Validate with separate fixtures for quota-blocked admission, admitted but
unscheduled pods, GPU type mismatch, pending storage, and denied node access.
Restricted visibility must yield an incomplete explanation rather than certainty.

### 2. GPU allocation and usage

Start with extended-resource requests, allocatable counts, published GPU labels,
sharing configuration when accessible, and workload links. Keep advertised
slots separate from physical devices and MIG instances. Exact device identity
requires driver/exporter evidence; pod requests alone are insufficient.

Then connect to an existing Prometheus endpoint for historical DCGM metrics:
utilization, device memory, hardware errors, and supported profiling signals.
Show collection coverage and attribution confidence. Lumen should not install
an exporter automatically or infer monetary savings from a momentary reading.

[NVIDIA sharing documentation](https://docs.nvidia.com/datacenter/cloud-native/gpu-operator/latest/gpu-sharing.html)
explains oversubscription and isolation differences.
[DCGM Exporter](https://docs.nvidia.com/datacenter/dcgm/latest/reference/command-line-reference/dcgm-exporter.html)
documents pod mapping and optional DRA metadata. These documents differ on
time-sharing attribution support, so discover exporter version/configuration
and validate the supported combinations rather than assuming universal mapping.
[Prometheus range queries](https://prometheus.io/docs/prometheus/latest/querying/api/)
provide history without a new Lumen storage backend.

Validate DRA and device-plugin paths, missing telemetry, MIG, shared devices,
stale series, and absent pod identity labels on real hardware before claiming
device-level correctness. Synthetic CRDs alone cannot establish that behavior.

### 3. AI workload lifecycle views

Recommend Ray as an initial candidate, contingent on the pilot team's stack.
Connect controller status to head/worker pods and existing investigation views.
Add Kubeflow TrainJob or KServe InferenceService adapters when users need them;
discover installed APIs and served versions instead of requiring every platform.

[Ray troubleshooting](https://docs.ray.io/en/latest/cluster/kubernetes/troubleshooting/troubleshooting.html)
shows why controller and pod state must be considered together.
[KServe debugging](https://kserve.github.io/website/docs/0.16/developer-guide/debugging)
illustrates model-download failures hidden behind service readiness.
The July 2026 [Headlamp Kubeflow plugin](https://kubernetes.io/blog/2026/07/13/introducing-headlamp-plugin-for-kubeflow/)
is a competitor precedent for API-driven ML operator views. Lumen's proposed
advantage is connecting workload state to devices and the existing incident flow.

### 4. Inference performance

Start with vLLM through existing telemetry: queue wait, first-token latency,
request rates, generated-token throughput, and available KV-cache signals.
Display each metric's supported version and scope. A Ready pod alone cannot
answer an application's latency question. Keep prompt bodies out of collection.

[vLLM production metrics](https://docs.vllm.ai/en/latest/usage/metrics/)
document the available signals and metric deprecation policy. GPU utilization
alone cannot identify the cause of slow inference; correlate observations and
retain uncertainty.

### 5. Distributed workload investigation

Extend existing selected-issue investigations with workers grouped by supported
job membership and per-worker logs/events. Highlight reported CUDA, OOM, or
communication errors as evidence, not automatic root-cause diagnoses. Active
checks need a separate explicit action and the existing context protection.

Begin with one Ray or training workload. Avoid a general NCCL profiler,
automatic remediation, or broad multi-vendor fabric diagnosis in the first release.

## Suggested sequence and validation

1. Deliver ordinary GPU scheduling explanations and device-plugin visibility.
   Add Kueue status when present; reuse existing DRA views.
2. Integrate DCGM/Prometheus with explicit history and attribution coverage.
3. Add one AI lifecycle adapter chosen from actual pilot environments.
4. Add inference metrics or distributed investigation according to pilot pain.

Recruit a small pilot using both dedicated and shared GPUs. Observe three tasks:
explain a waiting job, identify its allocation, and investigate low throughput.
Measure task completion, time, terminal workarounds, and incorrect conclusions.
Use those results to select the third adapter and later integrations.

Defer an ML experiment tracker, model registry, scheduler, automatic GPU
reconfiguration, broad plugin marketplace, and another AI assistant. These add
substantial product or operational scope without evidence that they solve the
selected operator workflow.
