import { invoke } from "@tauri-apps/api/core";
import type { GpuNode, GpuPod, Source } from "./gpuTypes";

export type GpuInventorySnapshot = {
  namespace: string;
  captured_at: string;
  pods: Source<GpuPod>;
  nodes: Source<GpuNode>;
};

export function fetchGpuInventory(context: string, namespace: string): Promise<GpuInventorySnapshot> {
  return invoke("gpu_inventory_snapshot", { context, namespace });
}
