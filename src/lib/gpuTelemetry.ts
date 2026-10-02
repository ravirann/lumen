import { gpuDeviceMappings } from "./gpuTelemetryMapping";
import type { DeviceSnapshot } from "./deviceResources";
import { invoke } from "@tauri-apps/api/core";
import type {
  GpuTelemetryConfig,
  GpuTelemetryCapabilities,
} from "./gpuTelemetryTypes";
function explicitContext(context: string) {
  if (!context.trim())
    throw new Error("An explicit cluster context is required.");
}
export async function getGpuTelemetryConfig(
  context: string,
): Promise<GpuTelemetryConfig | null> {
  explicitContext(context);
  return invoke("gpu_telemetry_config_get", { context });
}
export async function setGpuTelemetryConfig(
  context: string,
  config: GpuTelemetryConfig | null,
): Promise<void> {
  explicitContext(context);
  return invoke("gpu_telemetry_config_set", { context, config });
}
export async function getGpuTelemetryCapabilities(
  context: string,
): Promise<GpuTelemetryCapabilities> {
  explicitContext(context);
  return invoke("gpu_telemetry_capabilities", { context });
}
import type { GpuHistory, GpuSeries } from "./gpuTelemetryTypes";
import type { GpuPod } from "./gpuTypes";
export async function fetchGpuHistory(
  context: string,
  namespace: string | null,
  windowSeconds: number,
  endSeconds: number,
): Promise<GpuHistory> {
  explicitContext(context);
  return invoke("gpu_telemetry_history", {
    context,
    namespace,
    windowSeconds,
    endSeconds,
  });
}
export function attributeGpuSeries(
  series: GpuSeries,
  pod: GpuPod,
  shared: boolean,
  snapshot?: DeviceSnapshot,
): "verified" | "unverified" | "device-only" {
  const l = series.labels;
  if (
    shared ||
    (l.pod_uid && l.pod_uid !== pod.resource.uid) ||
    (l.namespace && l.namespace !== pod.resource.namespace) ||
    (l.pod && l.pod !== pod.resource.name)
  )
    return "device-only";
  if (!l.pod && !l.pod_uid) return "device-only";
  if (
    snapshot &&
    l.lumen_cluster_provenance === "verified" &&
    l.pod_uid === pod.resource.uid &&
    l.namespace === pod.resource.namespace &&
    l.container &&
    l.UUID &&
    (l.node || l.Hostname) &&
    (!l.node || l.node === pod.node_name) &&
    (!l.Hostname || l.Hostname === pod.node_name)
  ) {
    const matches = gpuDeviceMappings(pod, snapshot).filter(
      (m) =>
        m.uuid === l.UUID &&
        m.container === l.container &&
        m.node === pod.node_name,
    );
    if (matches.length === 1) return "verified";
  }
  // Exporter association is verified against current mapping only; history is not allocation proof.
  return "unverified";
}

export { gpuDeviceMappings } from "./gpuTelemetryMapping";
