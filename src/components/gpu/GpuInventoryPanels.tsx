import { useId, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";
import { SectionPanel } from "@/components/lumen/page";
import { Button } from "@/components/ui/button";
import {
  DevicePagination,
  useDevicePagination,
} from "@/components/devices/DevicePagination";
import { fetchGpuInventory, summarizeGpuRequests } from "@/lib/gpuInventory";
import type { Source } from "@/lib/gpuTypes";

function SourceNotice({
  source,
  label,
}: {
  source: Source<unknown>;
  label: string;
}) {
  const state =
    source.state === "available"
      ? source.complete
        ? "complete observed source"
        : "partial source; observations cover only returned items"
      : {
          forbidden: "access denied by cluster permissions",
          unsupported: "API not served by this cluster",
          error: "could not load resources",
        }[source.state];
  return (
    <p role="status" className="my-3 break-words text-xs text-text-muted">
      {label}: {state}. Captured: {source.captured_at || "time unavailable"}
      {source.message ? ` · ${source.message}` : ""}
    </p>
  );
}
function Quantities({ values }: { values: Record<string, number | null> }) {
  return (
    <ul className="space-y-1">
      {Object.entries(values).map(([key, value]) => (
        <li key={key} className="break-words">
          <span className="font-mono">{key}</span>:{" "}
          <span>{value === null ? "Unknown" : value}</span>
        </li>
      ))}
    </ul>
  );
}
export function GpuInventoryPanels({
  context,
  namespace,
}: {
  context: string;
  namespace: string;
}) {
  const [tab, setTab] = useState(0);
  const id = useId();
  const tabs = useRef<(HTMLButtonElement | null)[]>([]);
  const query = useQuery({
    queryKey: ["k8s", "gpu-inventory", context, namespace],
    queryFn: () => fetchGpuInventory(context, namespace),
    enabled: Boolean(context),
    retry: false,
    staleTime: 0,
  });
  // A failed refresh cannot turn a previous snapshot into current evidence.
  const data = query.isError ? undefined : query.data;
  const pods =
    data?.pods.state === "available"
      ? data.pods.items.filter(
          (p) => !["Succeeded", "Failed"].includes(p.phase),
        )
      : [];
  const nodes = data?.nodes.state === "available" ? data.nodes.items : [];
  const podPage = useDevicePagination(
    pods,
    JSON.stringify([context, namespace]),
  );
  const nodePage = useDevicePagination(
    nodes,
    JSON.stringify([context, namespace]),
  );
  const base = `/cluster/${encodeURIComponent(context)}`;
  return (
    <SectionPanel>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div
          role="tablist"
          aria-label="GPU inventory"
          className="flex flex-wrap gap-1"
        >
          {["GPU workloads", "GPU nodes"].map((label, i) => (
            <Button
              key={label}
              ref={(el) => {
                tabs.current[i] = el;
              }}
              role="tab"
              id={`${id}-tab-${i}`}
              aria-controls={`${id}-panel-${i}`}
              aria-selected={tab === i}
              tabIndex={tab === i ? 0 : -1}
              variant={tab === i ? "default" : "ghost"}
              size="sm"
              onClick={() => setTab(i)}
              onKeyDown={(event) => {
                let next = i;
                if (event.key === "ArrowRight" || event.key === "ArrowLeft")
                  next = 1 - i;
                else if (event.key === "Home") next = 0;
                else if (event.key === "End") next = 1;
                else return;
                event.preventDefault();
                setTab(next);
                tabs.current[next]?.focus();
              }}
            >
              {label}
            </Button>
          ))}
        </div>
        <Button
          size="sm"
          variant="ghost"
          disabled={!context || query.isFetching}
          onClick={() => void query.refetch()}
        >
          Refresh GPU inventory
        </Button>
      </div>
      <p className="my-3 text-xs text-text-muted">
        Effective active pod requests and node advertised slots are separate
        observations. Shared slots and MIG resource keys are not physical GPU
        counts. Scheduled pods identify a node, not a GPU UUID. DRA allocations
        remain separate below.
      </p>
      {!context ? (
        <p role="status">Select a cluster to inspect GPU inventory.</p>
      ) : query.isPending ? (
        <p role="status">Loading GPU inventory…</p>
      ) : query.isError ? (
        <p role="alert">
          Could not load GPU inventory. Retry to get current evidence.
        </p>
      ) : null}
      {data && query.isFetching && (
        <p role="status" className="my-2 text-xs text-text-muted">
          Refreshing; showing the previous captured observation.
        </p>
      )}
      {data && (
        <div
          role="tabpanel"
          id={`${id}-panel-${tab}`}
          aria-labelledby={`${id}-tab-${tab}`}
          tabIndex={0}
        >
          {tab === 0 ? (
            <>
              <h2 className="text-base font-semibold">
                {namespace
                  ? "Observed namespace requests"
                  : "Observed requests"}
              </h2>
              <SourceNotice source={data.pods} label="GPU workloads" />
              <p className="mb-3 text-xs text-text-muted">
                Scope: {namespace || "all namespaces"}. Terminal pods excluded.
                Unknown accounting stays unknown per resource key; collection is
                not an atomic snapshot.
              </p>
              {data.pods.state === "available" && (
                <>
                  <Quantities values={summarizeGpuRequests(pods)} />
                  {pods.length ? (
                    <div className="mt-3 overflow-x-auto">
                      <table className="w-full text-left text-xs">
                        <thead className="text-text-muted">
                          <tr>
                            {[
                              "Workload",
                              "Phase",
                              "Node relationship",
                              "Effective requests",
                              "DRA",
                            ].map((h) => (
                              <th key={h} className="p-2">
                                {h}
                              </th>
                            ))}
                          </tr>
                        </thead>
                        <tbody>
                          {podPage.items.map((p) => (
                            <tr
                              key={p.resource.uid}
                              className="border-t border-border-subtle"
                            >
                              <td className="p-2">
                                <Link
                                  className="text-accent-primary underline"
                                  to={`${base}/workloads/pods?${new URLSearchParams({ ns: p.resource.namespace || "", q: p.resource.name })}`}
                                >
                                  {p.resource.name}
                                </Link>
                                <p className="text-text-muted">
                                  {p.resource.namespace}
                                </p>
                              </td>
                              <td className="p-2">{p.phase}</td>
                              <td className="p-2">
                                {p.node_name || "Not scheduled"}
                              </td>
                              <td className="p-2">
                                <Quantities values={p.requests} />
                              </td>
                              <td className="p-2">
                                <Link
                                  className="text-accent-primary underline"
                                  aria-label={`DRA allocations for ${p.resource.name}`}
                                  to={`${base}/device-resources?${new URLSearchParams({ ns: p.resource.namespace || "", pod: p.resource.name })}`}
                                >
                                  Inspect claims
                                </Link>
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  ) : (
                    <p className="py-4 text-sm text-text-muted">
                      No GPU requests observed in this scope.
                    </p>
                  )}
                  <DevicePagination
                    pagination={podPage}
                    label="GPU workloads"
                  />
                </>
              )}
            </>
          ) : (
            <>
              <h2 className="text-base font-semibold">Advertised slots</h2>
              <SourceNotice source={data.nodes} label="GPU nodes" />
              <p className="mb-3 text-xs text-text-muted">
                Cluster-wide allocatable metadata; requests above use the
                selected namespace scope. Product and sharing labels are
                reported metadata. These observations do not establish free
                capacity.
              </p>
              {data.nodes.state === "available" && (
                <>
                  {nodes.length ? (
                    <div className="overflow-x-auto">
                      <table className="w-full text-left text-xs">
                        <thead className="text-text-muted">
                          <tr>
                            {[
                              "Node",
                              "Advertised allocatable slots",
                              "Reported metadata",
                            ].map((h) => (
                              <th key={h} className="p-2">
                                {h}
                              </th>
                            ))}
                          </tr>
                        </thead>
                        <tbody>
                          {nodePage.items.map((n) => (
                            <tr
                              key={n.resource.uid}
                              className="border-t border-border-subtle"
                            >
                              <td className="p-2">
                                <Link
                                  className="text-accent-primary underline"
                                  to={`${base}/device-resources?${new URLSearchParams({ ns: namespace, node: n.resource.name })}`}
                                >
                                  {n.resource.name} · DRA device details
                                </Link>
                              </td>
                              <td className="p-2">
                                <Quantities values={n.allocatable} />
                              </td>
                              <td className="p-2">
                                <ul>
                                  {Object.entries(n.gpu_labels).map(
                                    ([k, v]) => (
                                      <li className="break-words" key={k}>
                                        {k}: {v}
                                      </li>
                                    ),
                                  )}
                                </ul>
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  ) : (
                    <p className="py-4 text-sm text-text-muted">
                      No GPU slots advertised in the observed nodes.
                    </p>
                  )}
                  <DevicePagination pagination={nodePage} label="GPU nodes" />
                </>
              )}
            </>
          )}
        </div>
      )}
    </SectionPanel>
  );
}
