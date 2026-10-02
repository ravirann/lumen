import {
  object,
  objects,
  string,
  type DeviceSnapshot,
} from "./deviceResources";
import type { GpuPod } from "./gpuTypes";
export type GpuDeviceMapping = {
  uuid: string;
  node: string;
  container: string;
  shared: boolean;
};
/** Current DRA mapping validates exporter association, never past allocation/exclusivity. */
export function gpuDeviceMappings(
  pod: GpuPod,
  snapshot: DeviceSnapshot,
): GpuDeviceMapping[] {
  if (
    pod.resource.kind !== "Pod" ||
    snapshot.namespace !== pod.resource.namespace ||
    !pod.resource.uid ||
    !pod.node_name ||
    ["pods", "claims", "slices"].some(
      (k) => snapshot[k as "pods"].state !== "available",
    )
  )
    return [];
  const pods = snapshot.pods.items.filter((p) => {
    const m = object(p.metadata);
    return (
      m.uid === pod.resource.uid &&
      m.namespace === pod.resource.namespace &&
      m.name === pod.resource.name &&
      object(p.spec).nodeName === pod.node_name
    );
  });
  if (pods.length !== 1) return [];
  const p = pods[0],
    spec = object(p.spec),
    statuses = objects(object(p.status).resourceClaimStatuses);
  const containers = [
    "containers",
    "initContainers",
    "ephemeralContainers",
  ].flatMap((k) => objects(spec[k]));
  const tuples = snapshot.slices.items.flatMap((slice) => {
    const sp = object(slice.spec);
    if (
      sp.driver !== "gpu.nvidia.com" ||
      !string(sp.nodeName) ||
      !string(object(sp.pool).name)
    )
      return [];
    return objects(sp.devices).flatMap((d) => {
      const attrs = object(d.attributes ?? object(d.basic).attributes),
        type = string(object(attrs.type).string),
        uuid = string(object(attrs.uuid).string);
      if (!(
        (type === "gpu" && uuid.startsWith("GPU-")) ||
        (type === "mig" && uuid.startsWith("MIG-"))
      ))
        return [];
      return [
        {
          driver: string(sp.driver),
          pool: string(object(sp.pool).name),
          device: string(d.name),
          node: string(sp.nodeName),
          uuid,
        },
      ];
    });
  });
  const allResults = snapshot.claims.items.flatMap((c) =>
    objects(object(object(object(c.status).allocation).devices).results),
  );
  const mappings: GpuDeviceMapping[] = [];
  for (const ref of objects(spec.resourceClaims)) {
    const alias = string(ref.name),
      claimName =
        string(ref.resourceClaimName) ||
        string(statuses.find((s) => s.name === alias)?.resourceClaimName);
    if (!alias || !claimName) continue;
    const claims = snapshot.claims.items.filter(
      (c) =>
        object(c.metadata).name === claimName &&
        object(c.metadata).namespace === pod.resource.namespace,
    );
    if (claims.length !== 1) continue;
    const claim = claims[0],
      reserved = objects(object(claim.status).reservedFor);
    if (
      reserved.length !== 1 ||
      reserved[0].uid !== pod.resource.uid ||
      reserved[0].name !== pod.resource.name ||
      reserved[0].resource !== "pods" ||
      (reserved[0].apiGroup != null && reserved[0].apiGroup !== "")
    )
      continue;
    for (const r of objects(
      object(object(object(claim.status).allocation).devices).results,
    )) {
      if (
        r.driver !== "gpu.nvidia.com" ||
        !string(r.request)
      )
        continue;
      const shared =
        (r.adminAccess != null && r.adminAccess !== false) ||
        r.shareID != null ||
        r.shared === true;
      if (
        !shared &&
        allResults.filter(
          (x) =>
            x.driver === r.driver && x.pool === r.pool && x.device === r.device,
        ).length !== 1
      )
        continue;
      const candidates = tuples.filter(
        (t) =>
          t.driver === r.driver && t.pool === r.pool && t.device === r.device,
      );
      if (candidates.length !== 1) continue;
      const t = candidates[0];
      if (
        t.node !== pod.node_name ||
        tuples.filter((x) => x.uuid === t.uuid).length !== 1
      )
        continue;
      const users = containers.filter((c) =>
        objects(object(c.resources).claims).some(
          (x) =>
            x.name === alias &&
            (!x.request ||
              x.request === r.request ||
              x.request === string(r.request).split("/")[0]),
        ),
      );
      if (users.length !== 1 || !string(users[0].name)) continue;
      mappings.push({
        uuid: t.uuid,
        node: t.node,
        container: string(users[0].name),
        shared,
      });
    }
  }
  return mappings;
}

/** Association candidates reuse the sanitized DRA projection, without adding claim counts to slots. */
export function draUsagePods(snapshot: DeviceSnapshot): GpuPod[] {
  if (snapshot.pods.state !== "available") return [];
  return snapshot.pods.items.flatMap((p) => {
    const m = object(p.metadata), spec = object(p.spec);
    const containers = ["containers", "initContainers", "ephemeralContainers"]
      .flatMap(k => objects(spec[k]));
    if (
      m.namespace !== snapshot.namespace ||
      !string(m.uid) || !string(m.name) ||
      !objects(spec.resourceClaims).length ||
      !containers.some(c => objects(object(c.resources).claims).length)
    ) return [];
    return [{
      resource: {
        api_version: "v1", kind: "Pod", namespace: snapshot.namespace,
        name: string(m.name), uid: string(m.uid),
      },
      phase: string(object(p.status).phase),
      node_name: string(spec.nodeName) || null,
      requests: {}, owners: [],
    }];
  });
}
