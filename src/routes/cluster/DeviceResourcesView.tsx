import {
  DevicePagination,
  useDevicePagination,
} from "@/components/devices/DevicePagination";
import { useEffect, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Cpu, RefreshCw, Search } from "lucide-react";
import { NamespacePicker } from "@/components/NamespacePicker";
import { useNamespaceScope } from "@/hooks/useNamespaceScope";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { LumenPage, PageHeader, SectionPanel } from "@/components/lumen/page";
import {
  DevicePodLinks,
  DeviceSourceNotice,
  NodeDevicesPanel,
  PodDevicesPanel,
} from "@/components/devices/DevicePanels";
import {
  allocationDevices,
  buildDeviceInventory,
  fetchDeviceResources,
  object,
  objects,
  redactDeviceResource,
  resourceName,
  resourceNamespace,
  string,
  type DeviceCollection,
  type DeviceObject,
} from "@/lib/deviceResources";
import { GpuInventoryPanels } from "@/components/gpu/GpuInventoryPanels";
import { GpuUsagePanel } from "@/components/gpu/GpuUsagePanel";
import { cn } from "@/lib/utils";

const collections: {
  key: DeviceCollection;
  label: string;
  singular: string;
  description: string;
}[] = [
  {
    key: "claims",
    label: "Claims",
    singular: "ResourceClaim",
    description: "Namespaced requests and allocated devices.",
  },
  {
    key: "templates",
    label: "Templates",
    singular: "ResourceClaimTemplate",
    description: "Namespaced templates used to create pod claims.",
  },
  {
    key: "classes",
    label: "Classes",
    singular: "DeviceClass",
    description: "Cluster-wide device selection policies.",
  },
  {
    key: "slices",
    label: "Slices",
    singular: "ResourceSlice",
    description: "Cluster-wide inventory published by device drivers.",
  },
];
function summary(resource: DeviceObject, collection: DeviceCollection): string {
  const spec = object(resource.spec);
  if (collection === "claims") {
    const allocations = allocationDevices(resource);
    return allocations.length
      ? `${allocations.length} allocated device${allocations.length === 1 ? "" : "s"}`
      : "Not allocated";
  }
  if (collection === "slices")
    return `${string(spec.driver) || "Unknown driver"} · ${objects(spec.devices).length} published devices`;
  if (collection === "classes")
    return `${objects(spec.selectors).length} selector${objects(spec.selectors).length === 1 ? "" : "s"}`;
  return `${objects(object(object(spec.spec).devices).requests).length} device requests`;
}
export function DeviceResourcesView() {
  const { ctx = "" } = useParams();
  // React Router already decodes path parameters.
  const context = ctx;
  const [params, setParams] = useSearchParams();
  const scope = useNamespaceScope(
    context,
    params.has("ns") ? params.get("ns") : null,
  );
  const [collection, setCollection] = useState<DeviceCollection>("claims");
  const [usage, setUsage] = useState(false);
  const [filter, setFilter] = useState(() => params.get("q") ?? "");
  const requestedFilter = params.get("q") ?? "";
  useEffect(() => setFilter(requestedFilter), [requestedFilter, context, scope.namespace]);
  const [inspected, setInspected] = useState<{
    context: string;
    namespace: string;
    key: DeviceCollection;
    name: string;
    resourceNamespace: string;
  } | null>(null);
  const query = useQuery({
    queryKey: ["k8s", "device-resources", context, scope.namespace],
    queryFn: () => fetchDeviceResources(context, scope.namespace),
    enabled: Boolean(context) && !scope.isLoading,
    staleTime: 0,
    retry: false,
  });
  const data = query.data;
  const selected = collections.find((item) => item.key === collection)!;
  const source = data?.[collection];
  const rows =
    source?.state === "available"
      ? source.items.filter((resource) =>
          `${resourceName(resource)} ${resourceNamespace(resource)} ${summary(resource, collection)}`
            .toLowerCase()
            .includes(filter.toLowerCase()),
        )
      : [];
  const resourcePagination = useDevicePagination(
    rows,
    JSON.stringify([context, scope.namespace, collection, filter]),
  );
  const inspectionResource =
    inspected &&
    inspected.context === context &&
    inspected.namespace === scope.namespace
      ? data?.[inspected.key].items.find(
          (resource) =>
            resourceName(resource) === inspected.name &&
            resourceNamespace(resource) === inspected.resourceNamespace,
        )
      : undefined;
  const nodeName = params.get("node") || "";
  const podName = params.get("pod") || "";
  const nodes = useMemo(
    () =>
      data
        ? [
            ...new Set(
              buildDeviceInventory(data)
                .map((device) => device.placement)
                .filter(
                  (placement) =>
                    !["all nodes", "node selector", "unknown"].includes(
                      placement,
                    ),
                ),
            ),
          ].sort()
        : [],
    [data],
  );
  const nodesPagination = useDevicePagination(
    nodes,
    JSON.stringify([context, scope.namespace]),
  );
  const setNamespace = (namespace: string) => {
    const next = new URLSearchParams(params);
    next.set("ns", namespace);
    next.delete("pod");
    scope.setNamespace(namespace);
    setParams(next);
  };
  return (
    <LumenPage>
      <PageHeader
        eyebrow="GPU and dynamic resource allocation"
        title="Device resources"
        icon={<Cpu className="size-3.5" />}
        description="Inspect device requests, driver inventory, and reported workload health."
        actions={
          <div className="flex flex-wrap gap-2">
            <NamespacePicker
              value={scope.namespace}
              namespaces={scope.namespaces}
              onChange={setNamespace}
            />
            <Button
              size="sm"
              disabled={!context || scope.isLoading || query.isFetching}
              onClick={() => void query.refetch()}
            >
              <RefreshCw
                className={cn("size-3.5", query.isFetching && "animate-spin")}
              />
              Refresh
            </Button>
          </div>
        }
      />
      {scope.discoveryError != null && (
        <div
          role="status"
          className="rounded-control border border-warning/30 p-3 text-xs text-warning"
        >
          Namespace discovery is unavailable. Enter a namespace manually or
          select all namespaces.
        </div>
      )}
      {!context ? (
        <p role="alert">Select a cluster to inspect device resources.</p>
      ) : scope.isLoading ? (
        <p role="status" className="text-sm text-text-muted">
          Resolving namespace scope…
        </p>
      ) : query.isPending ? (
        <p role="status" className="text-sm text-text-muted">
          Loading device resources…
        </p>
      ) : null}
      {query.error && (
        <div
          role="alert"
          className="rounded-control border border-danger/30 p-3 text-sm text-danger"
        >
          Could not refresh device resources.{" "}
          {query.error instanceof Error
            ? query.error.message
            : String(query.error)}
          {data && (
            <p className="mt-1">
              Showing the previous snapshot. Retry to get current data.
            </p>
          )}
        </div>
      )}
      {context && !scope.isLoading && (
        <>
          <div
            role="group"
            aria-label="GPU view"
            className="flex flex-wrap gap-2"
          >
            <Button
              size="sm"
              aria-pressed={!usage}
              variant={!usage ? "default" : "ghost"}
              onClick={() => setUsage(false)}
            >
              GPU allocation
            </Button>
            <Button
              size="sm"
              aria-pressed={usage}
              variant={usage ? "default" : "ghost"}
              onClick={() => setUsage(true)}
            >
              Usage history
            </Button>
          </div>
          {usage ? (
            <GpuUsagePanel
              key={context}
              context={context}
              namespace={scope.namespace}
            />
          ) : (
            <GpuInventoryPanels context={context} namespace={scope.namespace} />
          )}
        </>
      )}
      {data && (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-text-muted">
            <span>
              Claims, templates and pods: {scope.namespace || "all namespaces"}{" "}
              · Classes and slices: cluster-wide
            </span>
            <span>
              Snapshot:{" "}
              {data.captured_at
                ? new Date(data.captured_at).toLocaleString()
                : "time unavailable"}
              {query.isFetching ? " · refreshing" : ""}
            </span>
          </div>
          <SectionPanel>
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div
                role="group"
                aria-label="Resource type"
                className="flex flex-wrap gap-1"
              >
                {collections.map((item) => (
                  <Button
                    key={item.key}
                    size="sm"
                    variant={collection === item.key ? "default" : "ghost"}
                    aria-pressed={collection === item.key}
                    onClick={() => setCollection(item.key)}
                  >
                    {item.label}
                    <span className="ml-1 text-xs">
                      {data[item.key].state === "available"
                        ? data[item.key].items.length
                        : "—"}
                    </span>
                  </Button>
                ))}
              </div>
              <div className="relative">
                <Search
                  aria-hidden="true"
                  className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-text-muted"
                />
                <Input
                  aria-label="Filter device resources"
                  className="w-56 pl-8"
                  placeholder="Filter resources…"
                  value={filter}
                  onChange={(event) => setFilter(event.target.value)}
                />
              </div>
            </div>
            <p className="my-3 text-xs text-text-muted">
              {selected.description}
            </p>
            {source && (
              <DeviceSourceNotice source={source} label={selected.label} />
            )}
            {source?.state === "available" &&
              (!rows.length ? (
                <p className="py-6 text-center text-sm text-text-secondary">
                  {filter
                    ? "No resources match this filter."
                    : `No ${selected.label.toLowerCase()} found in this scope.`}
                </p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full text-left text-xs">
                    <thead className="text-text-muted">
                      <tr>
                        <th className="p-2">Name</th>
                        <th className="p-2">Namespace</th>
                        <th className="p-2">Summary</th>
                        <th className="p-2">
                          <span className="sr-only">Inspect</span>
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {resourcePagination.items.map((resource) => (
                        <tr
                          key={`${resourceNamespace(resource)}/${resourceName(resource)}`}
                          className="border-t border-border-subtle"
                        >
                          <td className="p-2 font-medium text-text-primary">
                            {resourceName(resource)}
                          </td>
                          <td className="p-2 text-text-muted">
                            {resourceNamespace(resource) || "Cluster"}
                          </td>
                          <td className="p-2 text-text-secondary">
                            {summary(resource, collection)}
                          </td>
                          <td className="p-2 text-right">
                            <Button
                              variant="ghost"
                              size="sm"
                              aria-label={`Inspect ${selected.singular} ${resourceName(resource)}`}
                              onClick={() =>
                                setInspected({
                                  context,
                                  namespace: scope.namespace,
                                  key: collection,
                                  name: resourceName(resource),
                                  resourceNamespace:
                                    resourceNamespace(resource),
                                })
                              }
                            >
                              Inspect
                            </Button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ))}
            {source?.state === "available" && (
              <DevicePagination
                pagination={resourcePagination}
                label="resources"
              />
            )}
          </SectionPanel>
          {podName && (
            <PodDevicesPanel
              snapshot={data}
              podName={podName}
              namespace={scope.namespace}
            />
          )}
          {nodeName && (
            <NodeDevicesPanel
              key={context}
              snapshot={data}
              nodeName={nodeName}
            />
          )}
          <DevicePodLinks snapshot={data} context={context} />
          <SectionPanel>
            <h2 className="mb-2 text-base font-semibold">Node inventory</h2>
            <p className="mb-3 text-xs text-text-muted">
              Open a node to inspect published devices and pool completeness.
              Shared and selector-based devices may be accessible from multiple
              nodes.
            </p>
            <DeviceSourceNotice source={data.slices} label="Resource slices" />
            {nodes.length ? (
              <ul className="flex flex-wrap gap-2">
                {nodesPagination.items.map((node) => (
                  <li key={node}>
                    <Link
                      className="inline-flex rounded-control border border-border-default px-3 py-2 text-xs text-accent-primary hover:bg-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-primary"
                      to={`?${new URLSearchParams({ ns: scope.namespace, node })}`}
                    >
                      {node}
                    </Link>
                  </li>
                ))}
              </ul>
            ) : (
              data.slices.state === "available" && (
                <p className="text-sm text-text-muted">
                  No named nodes are advertised. Open a node’s device inventory
                  from its details to inspect shared or selector-based devices.
                </p>
              )
            )}
            {data.slices.state === "available" && (
              <DevicePagination pagination={nodesPagination} label="nodes" />
            )}
          </SectionPanel>
        </>
      )}
      <Dialog
        open={Boolean(inspectionResource)}
        onOpenChange={(open) => {
          if (!open) setInspected(null);
        }}
      >
        <DialogContent className="max-h-[85vh] max-w-3xl overflow-auto">
          <DialogHeader>
            <DialogTitle>
              {inspected
                ? collections.find((item) => item.key === inspected.key)
                    ?.singular
                : "Resource"}{" "}
              · {inspected?.name}
            </DialogTitle>
            <DialogDescription>
              Read-only resource JSON. Driver configuration, custom status data,
              and annotations are redacted.
            </DialogDescription>
          </DialogHeader>
          <pre
            className="overflow-auto rounded-control bg-elevated p-4 font-mono text-xs text-text-secondary"
            tabIndex={0}
          >
            {inspectionResource
              ? JSON.stringify(
                  redactDeviceResource(inspectionResource),
                  null,
                  2,
                )
              : ""}
          </pre>
        </DialogContent>
      </Dialog>
    </LumenPage>
  );
}
