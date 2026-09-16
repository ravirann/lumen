import { invoke, type ApplyOutcome, type AccessReviewResult } from "@/lib/k8s";

export type CrColumn = {
  name: string;
  type: string;
  description: string | null;
  json_path: string;
  priority: number;
};
export type CrCondition = {
  type: string;
  status: string;
  reason: string | null;
  message: string | null;
  observed_generation: number | null;
  last_transition_time: string | null;
};
export type CrdDetails = {
  name: string;
  group: string;
  kind: string;
  plural: string;
  scope: string;
  versions: {
    name: string;
    served: boolean;
    storage: boolean;
    columns: CrColumn[];
    schema: unknown | null;
  }[];
};
export type CustomResource = {
  name: string;
  namespace: string | null;
  age_seconds: number;
  status_hint: string | null;
  uid: string | null;
  resource_version: string | null;
  generation: number | null;
  conditions: CrCondition[];
  printer_cells: { name: string; value: unknown; supported: boolean }[];
};
export type CrTarget = {
  crd_name: string;
  version: string;
  namespace: string | null;
  name: string;
  uid?: string | null;
  resource_version?: string | null;
};
export type CustomResourceDetail = {
  yaml: string;
  uid: string | null;
  resource_version: string | null;
  generation: number | null;
  conditions: CrCondition[];
};

export const customResources = {
  details: (crdName: string, context: string) =>
    invoke<CrdDetails>("get_crd_details", { crdName, context }),
  list: (
    crdName: string,
    version: string,
    namespace: string | null,
    context: string,
  ) =>
    invoke<{ items: CustomResource[]; columns: CrColumn[] }>(
      "list_custom_resources",
      { crdName, version, namespace, context },
    ),
  get: (target: CrTarget, context: string) =>
    invoke<CustomResourceDetail>("get_custom_resource", { target, context }),
  access: (
    target: CrTarget,
    verb: "create" | "patch" | "delete",
    context: string,
  ) => invoke<AccessReviewResult>("check_cr_access", { target, verb, context }),
  write: (
    target: CrTarget,
    yaml: string,
    create: boolean,
    dryRun: boolean,
    context: string,
  ) =>
    invoke<ApplyOutcome>("write_cr", { target, yaml, create, dryRun, context }),
  delete: (target: CrTarget, context: string) =>
    invoke<void>("delete_cr", { target, context }),
};

export function resourceTemplate(
  crd: CrdDetails,
  version: string,
  namespace: string | null,
  name: string,
): string {
  // JSON strings are also valid YAML scalars, including quotes and newlines.
  return `apiVersion: ${JSON.stringify(`${crd.group}/${version}`)}\nkind: ${JSON.stringify(crd.kind)}\nmetadata:\n  name: ${JSON.stringify(name)}\n${namespace === null ? "" : `  namespace: ${JSON.stringify(namespace)}\n`}spec: {}\n`;
}

export function conditionFreshness(
  condition: CrCondition,
  generation: number | null,
): string {
  if (condition.observed_generation == null || generation == null)
    return "Freshness unknown";
  return condition.observed_generation < generation
    ? "Stale condition"
    : "Current generation";
}

export function printerCell(value: unknown): string {
  if (value === null || value === undefined) return "—";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}
