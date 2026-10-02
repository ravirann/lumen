import { invoke } from "@tauri-apps/api/core";
import type { SchedulingSnapshot } from "./gpuTypes";
export function fetchGpuScheduling(context: string, namespace: string, pod: string, expectedUid: string): Promise<SchedulingSnapshot> {
  return invoke("gpu_scheduling_snapshot", { context, namespace, pod, expectedUid });
}
