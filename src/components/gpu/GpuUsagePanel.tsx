import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { SectionPanel } from "@/components/lumen/page";
import { Button } from "@/components/ui/button";
import { fetchDeviceResources } from "@/lib/deviceResources";
import { fetchGpuInventory } from "@/lib/gpuInventory";
import {
  attributeGpuSeries,
  fetchGpuHistory,
  getGpuTelemetryCapabilities,
  getGpuTelemetryConfig,
} from "@/lib/gpuTelemetry";
import type {
  GpuMetricFamily,
  GpuSeries,
  GpuTelemetryConfig,
} from "@/lib/gpuTelemetryTypes";
import { GpuTelemetrySetup } from "./GpuTelemetrySetup";
const families: GpuMetricFamily[] = [
  "utilization",
  "framebuffer_used",
  "framebuffer_total",
  "sm_active",
  "tensor_active",
  "xid_errors",
];
const options = {
  retry: false as const,
  refetchOnWindowFocus: false,
  refetchOnReconnect: false,
  staleTime: 0,
};
export function GpuUsagePanel({
  context,
  namespace,
}: {
  context: string;
  namespace: string;
}) {
  const [savedRevision, setSavedRevision] = useState(0);
  const config = useQuery({
    ...options,
    queryKey: ["gpu-telemetry", "config", context],
    queryFn: () => getGpuTelemetryConfig(context),
    enabled: !!context,
  });
  return (
    <SectionPanel>
      {config.isPending ? (
        <p role="status">Loading telemetry configuration…</p>
      ) : config.isError ? (
        <p role="alert">
          Could not load telemetry configuration. Retry by reopening usage
          history.
        </p>
      ) : (
        <>
          <GpuTelemetrySetup
            context={context}
            onSaved={() => setSavedRevision((value) => value + 1)}
          />
          {config.data && (
            <ConfiguredUsage
              key={JSON.stringify([
                context,
                namespace,
                config.data,
                savedRevision,
              ])}
              context={context}
              namespace={namespace}
              config={config.data}
            />
          )}
        </>
      )}
    </SectionPanel>
  );
}
function ConfiguredUsage({
  context,
  namespace,
  config,
}: {
  context: string;
  namespace: string;
  config: GpuTelemetryConfig;
}) {
  const [windowSeconds, setWindow] = useState(3600);
  const [end, setEnd] = useState(() => Math.floor(Date.now() / 1000));
  const [podUid, setPodUid] = useState("");
  const source = JSON.stringify(config);
  const capability = useQuery({
    ...options,
    queryKey: ["gpu-telemetry", "capabilities", context, source],
    queryFn: () => getGpuTelemetryCapabilities(context),
  });
  const history = useQuery({
    ...options,
    queryKey: [
      "gpu-telemetry",
      "history",
      context,
      source,
      namespace,
      windowSeconds,
      end,
    ],
    queryFn: () =>
      fetchGpuHistory(context, namespace || null, windowSeconds, end),
    enabled: capability.data?.allowed === true && !capability.isError,
  });
  const inventory = useQuery({
    ...options,
    queryKey: ["gpu-telemetry", "inventory", context, source, namespace],
    queryFn: () => fetchGpuInventory(context, namespace),
  });
  const devices = useQuery({
    ...options,
    queryKey: ["gpu-telemetry", "mapping", context, source, namespace],
    queryFn: () => fetchDeviceResources(context, namespace),
  });
  const pods =
    !inventory.isError &&
    !inventory.isFetching &&
    inventory.data?.pods.state === "available"
      ? inventory.data.pods.items
      : [];
  const pod = pods.find((p) => p.resource.uid === podUid);
  const mapping =
    !devices.isError &&
    !devices.isFetching &&
    devices.data?.namespace === namespace
      ? devices.data
      : undefined;
  const available = capability.isError ? undefined : capability.data;
  const data = history.isError ? undefined : history.data;
  return (
    <div className="mt-5 space-y-3">
      <h2 className="text-base font-semibold">Usage history</h2>
      <p className="text-xs text-text-muted">
        {config.cluster_label
          ? `Cluster selector: ${config.cluster_label}=${config.cluster_value}`
          : "Single-cluster source: declared assumption, unverified."}{" "}
        Historical readings are exporter observations. Verified association uses
        the current DRA mapping, never past allocation proof or exclusive
        physical-device consumption.
      </p>
      {capability.isPending ? (
        <p role="status">Inspecting metric capabilities…</p>
      ) : capability.isError || !available?.allowed ? (
        <p role="alert">
          Service proxy access unavailable. Check services/proxy GET permission,
          Service connectivity and configured source.
        </p>
      ) : (
        <div className="text-xs text-text-muted">
          <p>
            Available families:{" "}
            {available.available_families.join(", ") || "none"}
          </p>
          <p>
            Missing families:{" "}
            {families
              .filter((f) => !available.available_families.includes(f))
              .join(", ") || "none"}
          </p>
          <p>
            Identity coverage:{" "}
            {available.identity_labels.join(", ") ||
              "none; device-only observations"}
          </p>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <label className="text-xs">
          Time window{" "}
          <select
            className="rounded-control border border-border-default bg-elevated p-2"
            value={windowSeconds}
            onChange={(e) => {
              setWindow(Number(e.target.value));
              setEnd(Math.floor(Date.now() / 1000));
            }}
          >
            {[
              [3600, "1 hour"],
              [21600, "6 hours"],
              [86400, "24 hours"],
              [604800, "7 days"],
            ].map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="text-xs">
          Current workload association{" "}
          <select
            className="max-w-full rounded-control border border-border-default bg-elevated p-2"
            value={podUid}
            onChange={(e) => setPodUid(e.target.value)}
          >
            <option value="">Device history (all returned series)</option>
            {pods.map((p) => (
              <option key={p.resource.uid} value={p.resource.uid}>
                {p.resource.namespace}/{p.resource.name}
              </option>
            ))}
          </select>
        </label>
        <Button
          size="sm"
          variant="ghost"
          disabled={!available?.allowed || history.isFetching}
          onClick={() => {
            const now = Math.floor(Date.now() / 1000);
            if (now === end) void history.refetch();
            else setEnd(now);
            void inventory.refetch();
            void devices.refetch();
          }}
        >
          Refresh history
        </Button>
      </div>
      {mapping && (
        <p className="text-xs text-text-muted">
          Current DRA mapping captured: {mapping.captured_at}. Association is
          checked against this observation only.
        </p>
      )}
      {(!mapping || !pods.length) && (
        <p className="text-xs text-text-muted">
          Current workload or device mapping unavailable. Historical device
          readings remain visible.
        </p>
      )}
      {history.isPending && available?.allowed && (
        <p role="status">Loading historical readings…</p>
      )}
      {history.isError && (
        <p role="alert">
          History unavailable. Check proxy access and source; try a narrower
          window or namespace scope for oversized results. Previous readings are
          unavailable until a successful refresh.
        </p>
      )}
      {data && (
        <>
          <p className="text-xs text-text-muted">
            Captured: {data.captured_at} ·{" "}
            {data.complete ? "Complete bounded response" : "Partial response"}
            {history.isFetching
              ? " · refreshing previous historical readings"
              : ""}
          </p>
          {data.warnings.length > 0 && (
            <p role="status" className="text-xs text-warning">
              Exporter warnings reported. XID signals are observations, not
              error counts or root-cause diagnoses.
            </p>
          )}
          {data.series.length === 0 ? (
            <p>No readings returned for this window.</p>
          ) : (
            data.series.map((series, i) => (
              <Metric
                key={i}
                series={series}
                attribution={
                  pod
                    ? attributeGpuSeries(
                        series,
                        pod,
                        Object.keys(pod.requests).some(
                          (k) => k === "nvidia.com/gpu.shared",
                        ),
                        mapping,
                      )
                    : "device-only"
                }
              />
            ))
          )}
        </>
      )}
    </div>
  );
}
function Metric({
  series,
  attribution,
}: {
  series: GpuSeries;
  attribution: string;
}) {
  const points = series.points;
  const finite = points.filter(
    (p): p is [number, number] => p[1] !== null && Number.isFinite(p[1]),
  );
  const min = Math.min(...finite.map((p) => p[1]));
  const max = Math.max(...finite.map((p) => p[1]));
  const start = points[0]?.[0] ?? 0;
  const end = points[points.length - 1]?.[0] ?? start;
  // Downsample only SVG display; native measurement points stay intact. Gaps split segments.
  const stride = Math.max(1, Math.ceil(points.length / 250));
  const segments: string[] = [];
  let segment: string[] = [];
  points.forEach(([time, value], i) => {
    if (value === null || !Number.isFinite(value)) {
      if (segment.length) segments.push(segment.join(" "));
      segment = [];
      return;
    }
    if (i % stride === 0 || i === points.length - 1) {
      segment.push(
        `${end === start ? 0 : ((time - start) / (end - start)) * 300},${max === min ? 30 : 55 - ((value - min) / (max - min)) * 50}`,
      );
    }
  });
  if (segment.length) segments.push(segment.join(" "));
  const unit = series.family.startsWith("framebuffer_")
    ? "bytes"
    : series.family === "xid_errors"
      ? "XID signal"
      : "%";
  const gaps = points.length - finite.length;
  return (
    <div className="rounded-control border border-border-subtle p-3">
      <h3 className="break-words text-sm font-medium">
        {series.labels.UUID ||
          series.labels.node ||
          series.labels.Hostname ||
          "Device identity unavailable"}
      </h3>
      <p className="text-xs text-text-muted">
        {series.family} · {attribution} ·{" "}
        {series.labels.namespace && `${series.labels.namespace}/`}
        {series.labels.pod || "no workload label"} · {gaps} gap
        {gaps === 1 ? "" : "s"}
      </p>
      <svg
        role="img"
        aria-label={`${series.family} historical readings in ${unit}; ${gaps} gaps`}
        viewBox="0 0 300 60"
        className="mt-2 h-16 w-full text-accent-primary"
        preserveAspectRatio="none"
      >
        {segments.map((s, i) => (
          <polyline
            key={i}
            points={s}
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            vectorEffect="non-scaling-stroke"
          />
        ))}
      </svg>
      <p className="text-xs">
        Latest: {points[points.length - 1]?.[1] ?? "Unavailable"} {unit} ·
        Range: {finite.length ? `${min}–${max} ${unit}` : "Unavailable"}
      </p>
    </div>
  );
}
