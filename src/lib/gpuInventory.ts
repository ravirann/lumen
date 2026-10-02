import { invoke } from "@tauri-apps/api/core";
import type { GpuNode, GpuPod, Source } from "./gpuTypes";

export type GpuInventorySnapshot = {
  namespace: string;
  captured_at: string;
  pods: Source<GpuPod>;
  nodes: Source<GpuNode>;
};

export function fetchGpuInventory(
  context: string,
  namespace: string,
): Promise<GpuInventorySnapshot> {
  return invoke("gpu_inventory_snapshot", { context, namespace });
}

/** Effective requests are computed natively; never mix DRA identities into slots. */
export function summarizeGpuRequests(
  pods: GpuPod[],
): Record<string, number | null> {
  const totals: Record<string, number | null> = {};
  for (const pod of pods) {
    if (["Succeeded", "Failed"].includes(pod.phase)) continue;
    for (const [key, amount] of Object.entries(pod.requests)) {
      const previous = totals[key] ?? 0;
      const sum = amount === null ? null : previous + amount;
      totals[key] =
        totals[key] === null ||
        amount === null ||
        !Number.isSafeInteger(amount) ||
        amount < 0 ||
        !Number.isSafeInteger(sum)
          ? null
          : sum;
    }
  }
  return totals;
}
