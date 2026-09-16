import { useMemo, useState } from "react";
import { useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Boxes, Plus, RefreshCw } from "lucide-react";
import { k8s, type CrdSummary } from "@/lib/k8s";
import {
  customResources,
  printerCell,
  type CrdDetails,
  type CustomResource,
} from "@/lib/customResources";
import { useNamespaceScope } from "@/hooks/useNamespaceScope";
import { useMutationCapability } from "@/hooks/useMutationCapability";
import { NamespacePicker } from "@/components/NamespacePicker";
import { CustomResourceDialog } from "@/components/CustomResourceDialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import { errorMessage } from "@/lib/errorMessage";

export function CrdBrowser() {
  const { ctx = "" } = useParams();
  const [params] = useSearchParams();
  const context = decodeURIComponent(ctx);
  // Context changes drop explorer state; namespace/version changes remount its resource pane.
  return (
    <Explorer
      key={context}
      context={context}
      requestedNamespace={params.get("ns")}
    />
  );
}

function Explorer({
  context,
  requestedNamespace,
}: {
  context: string;
  requestedNamespace: string | null;
}) {
  const scope = useNamespaceScope(context, requestedNamespace);
  const [params, setParams] = useSearchParams();
  const selectedScope = {
    ...scope,
    setNamespace: (next: string) => {
      scope.setNamespace(next);
      if (requestedNamespace !== null) {
        const updated = new URLSearchParams(params);
        updated.set("ns", next);
        setParams(updated, { replace: true });
      }
    },
  };
  const crds = useQuery({
    queryKey: ["k8s", "crds", context],
    queryFn: () => k8s.listCrds(context),
    staleTime: 60_000,
  });
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const selected = crds.data?.find((crd) => crd.name === selectedName);
  const groups = useMemo(() => {
    const filter = query.trim().toLowerCase();
    const grouped = new Map<string, CrdSummary[]>();
    for (const crd of crds.data ?? []) {
      if (
        filter &&
        ![crd.name, crd.kind, crd.group, ...crd.short_names].some((text) =>
          text.toLowerCase().includes(filter),
        )
      )
        continue;
      const group = grouped.get(crd.group) ?? [];
      group.push(crd);
      grouped.set(crd.group, group);
    }
    return [...grouped].sort(([a], [b]) => a.localeCompare(b));
  }, [crds.data, query]);
  return (
    <div className="flex h-full min-h-0 flex-col bg-app md:flex-row">
      <aside className="flex max-h-64 w-full shrink-0 flex-col border-b border-border-default bg-shell md:max-h-none md:w-64 md:border-b-0 md:border-r">
        <div className="flex items-center justify-between gap-2 p-3">
          <h1 className="flex items-center gap-2 text-sm font-semibold text-text-primary">
            <Boxes className="size-4" /> Custom resources
          </h1>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Refresh CRDs"
            disabled={crds.isFetching}
            onClick={() => void crds.refetch()}
          >
            <RefreshCw
              className={cn("size-4", crds.isFetching && "animate-spin")}
            />
          </Button>
        </div>
        <div className="px-3 pb-3">
          <Input
            aria-label="Filter CRDs"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Name, group, or kind…"
          />
        </div>
        <div className="min-h-0 flex-1 overflow-auto px-2 pb-3">
          {crds.error && (
            <p role="alert" className="p-2 text-xs text-danger">
              CRD discovery unavailable: {errorMessage(crds.error)}
            </p>
          )}
          {crds.isLoading ? (
            <p className="p-2 text-xs text-text-muted">Loading CRDs…</p>
          ) : groups.length === 0 && !crds.error ? (
            <p className="p-2 text-xs text-text-muted">No matching CRDs.</p>
          ) : (
            groups.map(([group, definitions]) => (
              <section key={group} className="mb-3">
                <h2
                  className="truncate px-2 py-1 text-xs text-text-muted"
                  title={group}
                >
                  {group}
                </h2>
                {definitions.map((crd) => (
                  <button
                    key={crd.name}
                    aria-label={`Select ${crd.kind}`}
                    onClick={() => setSelectedName(crd.name)}
                    aria-pressed={selectedName === crd.name}
                    className={cn(
                      "flex w-full items-center justify-between gap-2 rounded-control px-2 py-2 text-left text-sm hover:bg-hover",
                      selectedName === crd.name
                        ? "bg-accent-primary-soft text-accent-primary"
                        : "text-text-primary",
                    )}
                  >
                    <span className="truncate">{crd.kind}</span>
                    <span className="text-xs text-text-muted">
                      {crd.scope === "Namespaced" ? "NS" : "Cluster"}
                    </span>
                  </button>
                ))}
              </section>
            ))
          )}
        </div>
      </aside>
      <main className="min-h-0 min-w-0 flex-1 overflow-auto">
        {!selected ? (
          <div className="flex h-full min-h-48 flex-col items-center justify-center gap-2 p-6 text-center text-text-secondary">
            <Boxes className="size-6" />
            <p>Select a CRD to inspect and manage its custom resources.</p>
            <p className="text-xs">
              Discovery uses your current cluster permissions.
            </p>
          </div>
        ) : (
          <Definition
            key={`${selected.name}/${scope.namespace}`}
            context={context}
            summary={selected}
            scope={selectedScope}
          />
        )}
      </main>
    </div>
  );
}

function Definition({
  context,
  summary,
  scope,
}: {
  context: string;
  summary: CrdSummary;
  scope: ReturnType<typeof useNamespaceScope>;
}) {
  const details = useQuery({
    queryKey: ["k8s", "crd-details", context, summary.name],
    queryFn: () => customResources.details(summary.name, context),
    staleTime: 30_000,
  });
  const [version, setVersion] = useState(summary.preferred_version);
  const selectedVersion =
    details.data?.versions.find((item) => item.name === version && item.served)
      ?.name ?? details.data?.versions.find((item) => item.served)?.name;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border-default bg-shell p-4">
        <div className="min-w-0">
          <h2 className="text-lg font-semibold text-text-primary">
            {summary.kind}
          </h2>
          <p className="break-all text-xs text-text-muted">
            {summary.name} · {summary.scope}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-xs text-text-secondary">
            Version
            <select
              aria-label="CRD version"
              value={selectedVersion ?? ""}
              onChange={(event) => setVersion(event.target.value)}
              className="rounded-control border border-border-default bg-elevated px-2 py-2 text-text-primary"
            >
              {details.data?.versions
                .filter((item) => item.served)
                .map((item) => (
                  <option value={item.name} key={item.name}>
                    {item.name}
                    {item.storage ? " · storage" : ""}
                  </option>
                ))}
            </select>
          </label>
          {summary.scope === "Namespaced" && (
            <NamespacePicker
              value={scope.namespace}
              namespaces={scope.namespaces}
              onChange={scope.setNamespace}
            />
          )}
        </div>
      </div>
      {!!scope.discoveryError && summary.scope === "Namespaced" && (
        <p className="px-4 pt-3 text-xs text-warning">
          Namespace discovery unavailable. Choose or enter a known namespace to
          continue.
        </p>
      )}
      {details.error && (
        <div role="alert" className="p-4 text-sm text-danger">
          {errorMessage(details.error)}{" "}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void details.refetch()}
          >
            Retry definition
          </Button>
        </div>
      )}
      {details.isLoading && (
        <p className="p-4 text-sm text-text-muted">Loading definition…</p>
      )}
      {details.data && !selectedVersion && (
        <p className="p-4 text-sm text-warning">
          This CRD has no served versions.
        </p>
      )}
      {details.data && selectedVersion && (
        <Resources
          key={selectedVersion}
          context={context}
          crd={details.data}
          version={selectedVersion}
          namespace={scope.namespace}
          scopeLoading={scope.isLoading && summary.scope === "Namespaced"}
        />
      )}
    </div>
  );
}

function Resources({
  context,
  crd,
  version,
  namespace,
  scopeLoading,
}: {
  context: string;
  crd: CrdDetails;
  version: string;
  namespace: string;
  scopeLoading: boolean;
}) {
  const capability = useMutationCapability(context);
  const [query, setQuery] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [selected, setSelected] = useState<CustomResource | "create" | null>(
    null,
  );
  const resources = useQuery({
    queryKey: [
      "k8s",
      "custom-resources",
      context,
      crd.name,
      version,
      crd.scope === "Namespaced" ? namespace : null,
    ],
    queryFn: () =>
      customResources.list(
        crd.name,
        version,
        crd.scope === "Namespaced" ? namespace || null : null,
        context,
      ),
    enabled: !scopeLoading,
    staleTime: 10_000,
  });
  const columns = (resources.data?.columns ?? [])
    .map((column, index) => ({ ...column, index }))
    .filter((column) => advanced || column.priority === 0);
  const filter = query.trim().toLowerCase();
  const items = (resources.data?.items ?? []).filter(
    (item) =>
      !filter ||
      `${item.name} ${item.namespace ?? ""} ${item.status_hint ?? ""}`
        .toLowerCase()
        .includes(filter),
  );
  return (
    <>
      <div className="flex flex-wrap items-center gap-3 p-4">
        <Input
          className="max-w-xs"
          aria-label="Filter custom resources"
          placeholder="Filter name, namespace, or status…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <label className="flex items-center gap-2 text-xs text-text-secondary">
          <input
            type="checkbox"
            checked={advanced}
            onChange={(event) => setAdvanced(event.target.checked)}
          />{" "}
          Additional columns
        </label>
        <div className="ml-auto flex gap-2">
          <Button
            size="sm"
            variant="secondary"
            aria-label="Refresh resources"
            disabled={resources.isFetching}
            onClick={() => void resources.refetch()}
          >
            <RefreshCw
              className={cn("size-4", resources.isFetching && "animate-spin")}
            />{" "}
            Refresh
          </Button>
          <Button
            size="sm"
            disabled={capability.globalReadOnly}
            title={
              capability.globalReadOnly
                ? capability.reason
                : "Create a custom-resource instance"
            }
            onClick={() => setSelected("create")}
          >
            <Plus className="size-4" /> Create resource
          </Button>
        </div>
      </div>
      {resources.error && (
        <p role="alert" className="px-4 pb-3 text-sm text-danger">
          Resources unavailable: {errorMessage(resources.error)}
        </p>
      )}
      <details className="mx-4 mb-3 text-xs text-text-secondary">
        <summary className="cursor-pointer">
          CRD definition · schema and versions
        </summary>
        <p className="my-2">
          {crd.versions
            .map(
              (item) =>
                `${item.name}: ${item.served ? "served" : "not served"}${item.storage ? ", storage" : ""}`,
            )
            .join(" · ")}
        </p>
        <pre className="max-h-64 overflow-auto rounded-control bg-shell p-3">
          {JSON.stringify(
            crd.versions.find((item) => item.name === version)?.schema ??
              "No schema published.",
            null,
            2,
          )}
        </pre>
      </details>
      <div className="min-h-0 flex-1 overflow-auto">
        {resources.isLoading || scopeLoading ? (
          <p className="p-4 text-sm text-text-muted">Loading resources…</p>
        ) : items.length === 0 && !resources.error ? (
          <p className="p-6 text-sm text-text-secondary">
            {filter
              ? "No resources match the filter."
              : "No custom resources in this scope."}
          </p>
        ) : (
          items.length > 0 && (
            <table className="w-full text-left text-xs">
              <thead className="sticky top-0 bg-shell text-text-muted">
                <tr>
                  <th className="px-4 py-2">Name</th>
                  {crd.scope === "Namespaced" && (
                    <th className="px-4 py-2">Namespace</th>
                  )}
                  <th className="px-4 py-2">Status</th>
                  {columns.map((column) => (
                    <th
                      key={column.index}
                      className="px-4 py-2"
                      title={column.description ?? column.json_path}
                    >
                      {column.name}
                    </th>
                  ))}
                  <th className="px-4 py-2">Age</th>
                </tr>
              </thead>
              <tbody>
                {items.map((item) => (
                  <tr
                    key={item.uid ?? `${item.namespace}/${item.name}`}
                    className="border-t border-border-subtle text-text-secondary hover:bg-hover"
                  >
                    <td className="px-4 py-3">
                      <button
                        className="font-mono text-accent-primary underline-offset-4 hover:underline"
                        onClick={() => setSelected(item)}
                        aria-label={`Inspect ${item.name}`}
                      >
                        {item.name}
                      </button>
                    </td>
                    {crd.scope === "Namespaced" && (
                      <td className="px-4 py-3">{item.namespace}</td>
                    )}
                    <td className="px-4 py-3">
                      {item.status_hint ?? "Not reported"}
                    </td>
                    {columns.map((column) => {
                      const cell = item.printer_cells[column.index];
                      return (
                        <td
                          key={column.index}
                          className="max-w-64 truncate px-4 py-3"
                          title={
                            cell?.supported
                              ? printerCell(cell.value)
                              : "This column uses an unsupported JSONPath. Inspect YAML for the value."
                          }
                        >
                          {cell?.supported
                            ? printerCell(cell.value)
                            : "Unsupported path"}
                        </td>
                      );
                    })}
                    <td className="px-4 py-3 tabular-nums">
                      {formatAge(item.age_seconds)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )
        )}
      </div>
      {resources.dataUpdatedAt > 0 && (
        <p className="border-t border-border-subtle px-4 py-2 text-xs text-text-muted">
          {resources.data?.items.length ?? 0} resources · fetched{" "}
          {new Date(resources.dataUpdatedAt).toLocaleTimeString()} ·
          controller-reported status
        </p>
      )}
      {selected && (
        <CustomResourceDialog
          key={
            selected === "create"
              ? "create"
              : `${selected.uid}/${selected.name}`
          }
          context={context}
          crd={crd}
          version={version}
          namespace={namespace}
          resource={selected === "create" ? undefined : selected}
          onClose={() => setSelected(null)}
        />
      )}
    </>
  );
}

function formatAge(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86400)}d`;
}
