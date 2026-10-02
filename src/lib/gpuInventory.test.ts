import { describe, expect, it } from "vitest";
import { summarizeGpuRequests } from "./gpuInventory";
import type { GpuPod } from "./gpuTypes";
const pod = (requests: GpuPod["requests"], phase = "Running"): GpuPod => ({
  resource: {
    api_version: "v1",
    kind: "Pod",
    namespace: "team",
    name: "train",
    uid: "u1",
  },
  phase,
  node_name: "gpu-node",
  owners: [],
  requests,
});
describe("summarizeGpuRequests", () => {
  it("keeps MIG and shared slots in separate request totals", () => {
    expect(
      summarizeGpuRequests([
        pod({ "nvidia.com/gpu.shared": 8 }),
        pod({ "nvidia.com/mig-1g.5gb": 2 }),
      ]),
    ).toEqual({ "nvidia.com/gpu.shared": 8, "nvidia.com/mig-1g.5gb": 2 });
  });
  it("excludes terminal pods and propagates unknown per key", () => {
    expect(
      summarizeGpuRequests([
        pod({ gpu: 2 }),
        pod({ gpu: null }),
        pod({ gpu: 10 }, "Succeeded"),
        pod({ shared: 4 }, "Failed"),
      ]),
    ).toEqual({ gpu: null });
  });
  it("rejects unsafe, negative, fractional and overflowing sums", () => {
    expect(
      summarizeGpuRequests([
        pod({ a: Number.MAX_SAFE_INTEGER, b: -1, c: 0.5 }),
        pod({ a: 1 }),
      ]),
    ).toEqual({ a: null, b: null, c: null });
    expect(summarizeGpuRequests([])).toEqual({});
  });
});
