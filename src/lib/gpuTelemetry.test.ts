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
import type { DeviceSnapshot } from "./deviceResources";
function mappingFixture(): DeviceSnapshot {
  const source = (items: Record<string, unknown>[]) => ({
    state: "available" as const,
    items,
    message: null,
  });
  return {
    namespace: "team",
    captured_at: "now",
    templates: source([]),
    classes: source([]),
    pods: source([
      {
        metadata: { namespace: "team", name: "train", uid: "u1" },
        spec: {
          nodeName: "gpu",
          resourceClaims: [{ name: "gpu-claim", resourceClaimName: "claim" }],
          containers: [
            {
              name: "worker",
              resources: { claims: [{ name: "gpu-claim", request: "req" }] },
            },
          ],
        },
      },
    ]),
    claims: source([
      {
        metadata: { namespace: "team", name: "claim", uid: "claim-uid" },
        status: {
          reservedFor: [{ resource: "pods", name: "train", uid: "u1" }],
          allocation: {
            devices: {
              results: [
                {
                  driver: "gpu.nvidia.com",
                  pool: "pool",
                  device: "one",
                  request: "req",
                },
              ],
            },
          },
        },
      },
    ]),
    slices: source([
      {
        spec: {
          driver: "gpu.nvidia.com",
          nodeName: "gpu",
          pool: { name: "pool" },
          devices: [
            {
              name: "one",
              attributes: {
                type: { string: "gpu" },
                uuid: { string: "GPU-1" },
              },
            },
          ],
        },
      },
    ]),
  };
}
const mappedSeries = {
  ...series,
  labels: {
    ...series.labels,
    container: "worker",
    node: "gpu",
    lumen_cluster_provenance: "verified",
  },
};
describe("DRA-backed association", () => {
  it("verifies exact exporter association against current authoritative DRA mapping", () =>
    expect(attributeGpuSeries(mappedSeries, pod, false, mappingFixture())).toBe(
      "verified",
    ));
  it.each(["pods", "claims", "slices"] as const)(
    "requires available %s evidence",
    (key) => {
      const s = mappingFixture();
      s[key].state = "forbidden";
      expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe(
        "unverified",
      );
    },
  );
  it("rejects duplicate slice mappings", () => {
    const s = mappingFixture();
    s.slices.items.push(s.slices.items[0]);
    expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe("unverified");
  });
  it("keeps known shared allocations device-only", () => {
    const s = mappingFixture();
    const c = s.claims.items[0] as any;
    c.status.allocation.devices.results[0].shareID = "shared";
    expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe("device-only");
  });
  it("requires reserved UID and matching container request", () => {
    for (const change of ["uid", "request"]) {
      const s = mappingFixture();
      if (change === "uid")
        (s.claims.items[0] as any).status.reservedFor[0].uid = "replaced";
      else
        (
          s.pods.items[0] as any
        ).spec.containers[0].resources.claims[0].request = "wrong";
      expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe(
        "unverified",
      );
    }
  });
  it("never substitutes MIG parent UUID", () => {
    const s = mappingFixture();
    const a = (s.slices.items[0] as any).spec.devices[0].attributes;
    a.type.string = "mig";
    a.uuid.string = "MIG-1";
    a.parentUUID = { string: "GPU-1" };
    expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe("unverified");
    expect(
      attributeGpuSeries(
        { ...mappedSeries, labels: { ...mappedSeries.labels, UUID: "MIG-1" } },
        pod,
        false,
        s,
      ),
    ).toBe("verified");
  });
});
describe("mapping evidence completeness", () => {
  it.each([
    "container",
    "UUID",
    "pod_uid",
    "namespace",
    "node",
    "lumen_cluster_provenance",
  ])("missing %s cannot verify", (key) => {
    const labels: Record<string, string> = { ...mappedSeries.labels };
    delete labels[key];
    expect(
      attributeGpuSeries(
        { ...mappedSeries, labels },
        pod,
        false,
        mappingFixture(),
      ),
    ).not.toBe("verified");
  });
  it("requires selected namespace and node", () => {
    const s = mappingFixture();
    s.namespace = "other";
    expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe("unverified");
    s.namespace = "team";
    (s.slices.items[0] as any).spec.nodeName = "other";
    expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe("unverified");
  });
  it("rejects noncore reservations and labels known admin access device-only", () => {
    for (const field of ["apiGroup", "adminAccess"]) {
      const s = mappingFixture();
      const c = s.claims.items[0] as any;
      if (field === "apiGroup") c.status.reservedFor[0].apiGroup = "foreign";
      else c.status.allocation.devices.results[0].adminAccess = true;
      expect(attributeGpuSeries(mappedSeries, pod, false, s)).toBe(
        field === "adminAccess" ? "device-only" : "unverified",
      );
    }
  });
});

import { normalizeGpuPoint } from "./gpuTelemetry";
it("retains unsupported chart readings as gaps rather than idle GPUs", () => {
  expect(normalizeGpuPoint("NaN")).toBeNull();
  expect(normalizeGpuPoint("+Inf")).toBeNull();
  expect(normalizeGpuPoint("")).toBeNull();
  expect(normalizeGpuPoint("0")).toBe(0);
});
it("rejects JavaScript-only numeric coercions unsupported by the native parser", () => {
  expect(normalizeGpuPoint("0x10")).toBeNull();
  expect(normalizeGpuPoint(" 1 ")).toBeNull();
  expect(normalizeGpuPoint("1e2")).toBe(100);
});

it.each<Record<string, string>>([{GPU_I_ID: "7"}, {GPU_I_PROFILE: "1g.5gb"}, {GPU_I_ID: "7", GPU_I_PROFILE: "1g.5gb"}])("rejects unreconciled instance qualifiers %j", (qualifiers) => {
  expect(attributeGpuSeries({...mappedSeries, labels: {...mappedSeries.labels, ...qualifiers}}, pod, false, mappingFixture())).toBe("unverified");
});

it("rejects conflicting qualifiers even with an exact MIG UUID", () => {
  const s = mappingFixture();
  const attrs = (s.slices.items[0] as any).spec.devices[0].attributes;
  attrs.type.string = "mig"; attrs.uuid.string = "MIG-1";
  expect(attributeGpuSeries({...mappedSeries, labels: {...mappedSeries.labels, UUID: "MIG-1", GPU_I_ID: "7", GPU_I_PROFILE: "conflicting"}}, pod, false, s)).toBe("unverified");
});
it("unknown shared identity remains unverified", () => {
  const s = mappingFixture();
  (s.claims.items[0] as any).status.allocation.devices.results[0].shareID = "shared";
  expect(attributeGpuSeries({...mappedSeries, labels: {...mappedSeries.labels, UUID: "GPU-unknown"}}, pod, false, s)).toBe("unverified");
});

import { draUsagePods } from "./gpuTelemetryMapping";
it("DRA candidates require available same-namespace UID-scoped sanitized pods", () => {
  const s = mappingFixture();
  expect(draUsagePods(s)).toMatchObject([{resource: {uid: "u1"}, requests: {}}]);
  s.namespace = "other";
  expect(draUsagePods(s)).toEqual([]);
  s.namespace = "team"; s.pods.state = "forbidden";
  expect(draUsagePods(s)).toEqual([]);
});
