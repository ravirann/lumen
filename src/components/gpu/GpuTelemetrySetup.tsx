import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  getGpuTelemetryConfig,
  getGpuTelemetryCapabilities,
  setGpuTelemetryConfig,
} from "@/lib/gpuTelemetry";
import type {
  GpuTelemetryCapabilities,
  GpuTelemetryConfig,
} from "@/lib/gpuTelemetryTypes";
export function GpuTelemetrySetup({
  context,
  onSaved,
}: {
  context: string;
  onSaved: () => void | Promise<void>;
}) {
  const client = useQueryClient();
  const query = useQuery({
    queryKey: ["gpu-telemetry", "config", context],
    queryFn: () => getGpuTelemetryConfig(context),
    enabled: !!context,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  return query.isPending ? (
    <p role="status">Loading telemetry configuration…</p>
  ) : query.isError ? (
    <p role="alert">Could not load telemetry configuration.</p>
  ) : (
    <SourceForm
      key={JSON.stringify([context, query.data])}
      context={context}
      initial={query.data ?? null}
      onTested={(capabilities) =>
        client.setQueryData(
          [
            "gpu-telemetry",
            "capabilities",
            context,
            JSON.stringify(query.data),
          ],
          capabilities,
        )
      }
      onSaved={async () => {
        await client.cancelQueries({
          predicate: (q) =>
            q.queryKey[0] === "gpu-telemetry" && q.queryKey[2] === context,
        });
        client.removeQueries({
          queryKey: ["gpu-telemetry", "history", context],
        });
        client.removeQueries({
          queryKey: ["gpu-telemetry", "capabilities", context],
        });
        await client.invalidateQueries({
          queryKey: ["gpu-telemetry", "config", context],
        });
        await onSaved();
      }}
    />
  );
}
function SourceForm({
  context,
  initial,
  onSaved,
  onTested,
}: {
  context: string;
  initial: GpuTelemetryConfig | null;
  onSaved: () => void | Promise<void>;
  onTested: (capabilities: GpuTelemetryCapabilities) => void;
}) {
  const [config, setConfig] = useState<GpuTelemetryConfig>(
    initial ?? {
      namespace: "",
      service: "",
      port: "",
      cluster_label: null,
      cluster_value: null,
      single_cluster_acknowledged: false,
    },
  );
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const valid = !!(
    config.namespace.trim() &&
    config.service.trim() &&
    config.port.trim() &&
    ((config.cluster_label?.trim() && config.cluster_value?.trim()) ||
      (!config.cluster_label &&
        !config.cluster_value &&
        config.single_cluster_acknowledged))
  );
  async function save(remove = false) {
    setBusy(true);
    setError("");
    try {
      await setGpuTelemetryConfig(context, remove ? null : config);
      await onSaved();
      setStatus(
        remove ? "Source removed." : "Source saved. Test the saved connection.",
      );
    } catch {
      setError(
        "Could not save telemetry source. Check Service fields and cluster provenance, then retry.",
      );
    } finally {
      setBusy(false);
    }
  }
  async function test() {
    setBusy(true);
    setError("");
    try {
      const c = await getGpuTelemetryCapabilities(context);
      onTested(c);
      setStatus(
        c.allowed
          ? `Connection available. Metric families: ${c.available_families.join(", ") || "none"}.`
          : "Service proxy access unavailable. Check services/proxy GET permission and Service connectivity.",
      );
    } catch {
      setError(
        "Could not test saved source. Check services/proxy GET permission and Service connectivity.",
      );
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="space-y-3">
      <h2 className="text-base font-semibold">
        Configure existing Prometheus Service
      </h2>
      <p className="text-xs text-text-muted">
        Uses this context’s Kubernetes credentials and services/proxy GET
        permission. Service proxy access can expose the configured Prometheus
        dataset. Namespace filters scope queries, not authorization inside
        Prometheus. Configuration stores only non-secret Service fields and
        cluster provenance.
      </p>
      <div className="grid gap-3 sm:grid-cols-2">
        {(
          [
            "namespace",
            "service",
            "port",
            "cluster_label",
            "cluster_value",
          ] as const
        ).map((field, i) => (
          <label className="space-y-1 text-xs" key={field}>
            {
              [
                "Service namespace",
                "Service name",
                "Service port",
                "Cluster label",
                "Cluster value",
              ][i]
            }
            <Input
              value={config[field] ?? ""}
              onChange={(e) =>
                setConfig({
                  ...config,
                  [field]:
                    e.target.value ||
                    (field.startsWith("cluster_") ? null : ""),
                })
              }
            />
          </label>
        ))}
      </div>
      <label className="flex items-start gap-2 text-xs">
        <input
          type="checkbox"
          checked={config.single_cluster_acknowledged}
          onChange={(e) =>
            setConfig({
              ...config,
              single_cluster_acknowledged: e.target.checked,
            })
          }
        />
        This source contains only this cluster (declared assumption,
        unverified).
      </label>
      <p className="text-xs text-text-muted">
        For multi-cluster sources enter an exact cluster label and value. No
        PromQL, credentials, arbitrary URLs or exporter installation.
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" disabled={!valid || busy} onClick={() => void save()}>
          Save source
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={!initial || busy}
          onClick={() => void test()}
        >
          Test saved source
        </Button>
        {initial && (
          <Button
            size="sm"
            variant="ghost"
            disabled={busy}
            onClick={() => void save(true)}
          >
            Remove source
          </Button>
        )}
      </div>
      {error && <p role="alert">{error}</p>}
      {status && <p role="status">{status}</p>}
    </div>
  );
}
