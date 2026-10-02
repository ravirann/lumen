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
  // Inventory currently has no authoritative device/container allocation mapping.
  // Exporter labels alone cannot establish an exclusive device-to-pod association.
  return "unverified";
}
