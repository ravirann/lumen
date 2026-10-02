import { describe, it, expect } from "vitest";
import { attributeGpuSeries } from "./gpuTelemetry";
const pod = {
  resource: {
    api_version: "v1",
    kind: "Pod",
    namespace: "team",
    name: "train",
    uid: "u1",
  },
  phase: "Running",
  node_name: "gpu",
  owners: [],
  requests: {},
};
const series = {
  family: "utilization" as const,
  labels: { UUID: "GPU-1", namespace: "team", pod: "train", pod_uid: "u1" },
  points: [],
};
describe("GPU attribution", () => {
  it("does not assign shared-device activity exclusively to a pod", () =>
    expect(attributeGpuSeries(series, pod, true)).toBe("device-only"));
  it("requires trusted cluster and device mapping", () =>
    expect(attributeGpuSeries(series, pod, false)).toBe("unverified"));
  it("keeps mismatched identities device-only", () =>
    expect(
      attributeGpuSeries(
        { ...series, labels: { ...series.labels, pod_uid: "other" } },
        pod,
        false,
      ),
    ).toBe("device-only"));
});
