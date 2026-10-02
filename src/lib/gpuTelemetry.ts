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
