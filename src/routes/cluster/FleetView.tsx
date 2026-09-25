import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import {
  AlertTriangle,
  ArchiveRestore,
  Bot,
  ChevronDown,
  CircleDot,
  Cloud,
  Cpu,
  Database,
  Gauge,
  Layers3,
  MemoryStick,
  Plus,
  RefreshCw,
  Server,
  ShieldAlert,
  Tag,
  Wrench,
  Trash2,
  X,
} from "lucide-react";
import { k8s, type ContextInfo, type DeletedContextSummary, type FleetCard } from "@/lib/k8s";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Card as UiCard, CardHeader, CardTitle } from "@/components/ui/card";
import {
  DataTable,
  DataTableBody,
  DataTableCell,
  DataTableHead,
  DataTableHeader,
  DataTableRow,
  DataTableShell,
} from "@/components/ui/data-table";
import { StatusBadge } from "@/components/ui/status-badge";
import { LumenPage, PageHeader, PanelHeading, SectionPanel } from "@/components/lumen/page";
import { MetricCard, type MetricTone } from "@/components/lumen/metric-card";
import { useClusterStore } from "@/state/cluster";
import { ConnectionDiagnosticsDialog } from "@/components/ConnectionDiagnosticsDialog";
import { classifyConnectionError } from "@/lib/connectionDiagnostics";

function pct(n: number | null): string {
  if (n === null) return "—";
  return `${Math.round(n)}%`;
}
function heatBand(n: number | null): string {
  if (n === null) return "bg-term-panel-2";
  if (n < 50) return "bg-success/70";
  if (n < 75) return "bg-term-green/80";
  if (n < 90) return "bg-warning";
  return "bg-term-red";
}

/**
 * Friendly label + tone for a detected K8s distribution. Backend emits
 * stable lowercase ids; map them here to short display labels and a
 * coarse color hint. Unknown / unset distributions render no badge.
 */
const DISTRIBUTION_LABELS: Record<string, string> = {
  eks: "EKS",
  gke: "GKE",
  aks: "AKS",
  openshift: "OpenShift",
  k3s: "k3s",
  kind: "kind",
  minikube: "minikube",
  "docker-desktop": "Docker Desktop",
  rancher: "Rancher",
};

function DistributionBadge({ value }: { value: string | undefined }) {
  if (!value) return null;
  const label = DISTRIBUTION_LABELS[value] ?? value;
  return (
    <span
      className="inline-flex items-center rounded border border-border-subtle bg-elevated px-1.5 py-0.5 text-[10px] font-mono text-text-secondary"
      title={`detected distribution: ${label}`}
    >
      {label}
    </span>
  );
}

function fleetRiskScore(card: FleetCard): number {
  if (!card.reachable) return 0;
  if (card.error) return 4;
  if (card.context.is_prod && card.health.pods_failed > 0) return 1;
  if (card.health.pods_failed > 0) return 2;
  if ((card.cpu_percent ?? 0) >= 90 || (card.mem_percent ?? 0) >= 90) return 3;
  if (card.health.pods_pending > 0) return 4;
  return 5;
}

type FleetEntry = {
  context: ContextInfo;
  card?: FleetCard;
  connecting: boolean;
};

const FLEET_CARDS_STORAGE_KEY = "lumen:fleet:connected-cards";
const FLEET_HIDDEN_STORAGE_KEY = "lumen:fleet:hidden-contexts";
const FLEET_LABELS_STORAGE_KEY = "lumen:fleet:cluster-labels";

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object" && "message" in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return String(err);
}

function readStoredCards(): Record<string, FleetCard> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.sessionStorage.getItem(FLEET_CARDS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, FleetCard>)
      : {};
  } catch {
    return {};
  }
}

function writeStoredCards(cards: Record<string, FleetCard>): void {
  if (typeof window === "undefined") return;
  try {
    if (Object.keys(cards).length === 0) {
      window.sessionStorage.removeItem(FLEET_CARDS_STORAGE_KEY);
    } else {
      window.sessionStorage.setItem(FLEET_CARDS_STORAGE_KEY, JSON.stringify(cards));
    }
  } catch {
    // storage is an optimization; ignore quota/private-mode failures.
  }
}

function readStoredHidden(): Set<string> {
  if (typeof window === "undefined") return new Set();
  try {
    const raw = window.sessionStorage.getItem(FLEET_HIDDEN_STORAGE_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((name): name is string => typeof name === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

function writeStoredHidden(hidden: Set<string>): void {
  if (typeof window === "undefined") return;
  try {
    if (hidden.size === 0) {
      window.sessionStorage.removeItem(FLEET_HIDDEN_STORAGE_KEY);
    } else {
      window.sessionStorage.setItem(FLEET_HIDDEN_STORAGE_KEY, JSON.stringify([...hidden]));
    }
  } catch {
    // storage is an optimization; ignore quota/private-mode failures.
  }
}

function readStoredLabels(): Record<string, string[]> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(FLEET_LABELS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed)
        .map(([name, labels]) => [
          name,
          Array.isArray(labels)
            ? labels.filter((label): label is string => typeof label === "string")
            : [],
        ])
        .filter(([, labels]) => labels.length > 0),
    );
  } catch {
    return {};
  }
}

function writeStoredLabels(labels: Record<string, string[]>): void {
  if (typeof window === "undefined") return;
  try {
    const compact = Object.fromEntries(
      Object.entries(labels).filter(([, values]) => values.length > 0),
    );
    if (Object.keys(compact).length === 0) {
      window.localStorage.removeItem(FLEET_LABELS_STORAGE_KEY);
    } else {
      window.localStorage.setItem(FLEET_LABELS_STORAGE_KEY, JSON.stringify(compact));
    }
  } catch {
    // Labels are local UI metadata; ignore quota/private-mode failures.
  }
}

function entryRiskScore(entry: FleetEntry): number {
  if (entry.card) return fleetRiskScore(entry.card);
  if (entry.connecting) return 6;
  return 7;
}

function average(values: Array<number | null | undefined>): number | null {
  const usable = values.filter((value): value is number => typeof value === "number");
  if (usable.length === 0) return null;
  return usable.reduce((sum, value) => sum + value, 0) / usable.length;
}

function formatNumber(value: number): string {
  return new Intl.NumberFormat("en", { maximumFractionDigits: 0 }).format(value);
}

function MetricTile({
  icon,
  label,
  value,
  sub,
  tone = "info",
  trend = false,
}: {
  icon: React.ReactNode;
  label: string;
  value: React.ReactNode;
  sub: React.ReactNode;
  tone?: "good" | "warn" | "bad" | "info";
  trend?: boolean;
}) {
  const mappedTone: MetricTone =
    tone === "good" ? "success" : tone === "warn" ? "warning" : tone === "bad" ? "error" : "info";
  return (
    <MetricCard
      icon={icon}
      label={label}
      value={value}
      helper={sub}
      tone={mappedTone}
      trend={trend}
    />
  );
}

function CapacityBar({ value }: { value: number | null }) {
  return (
    <div className="flex items-center gap-2">
      <div className="h-1.5 w-20 overflow-hidden rounded-full bg-elevated">
        <div
          className={cn("h-full rounded-full", heatBand(value))}
          style={{ width: `${value === null ? 0 : Math.min(100, Math.max(0, value))}%` }}
        />
      </div>
      <span className="w-9 text-right text-[11px] text-text-secondary tabular-nums">{pct(value)}</span>
    </div>
  );
}

function ClusterHealthTable({ cards }: { cards: FleetCard[] }) {
  const rows = cards.slice(0, 8);
  if (rows.length === 0) {
    return (
      <UiCard className="p-6 text-sm text-text-secondary shadow-none">
        Connect clusters to populate live health, capacity, and workload data.
      </UiCard>
    );
  }
  return (
    <UiCard className="overflow-hidden shadow-none">
      <CardHeader>
        <div>
          <CardTitle>Cluster Health</CardTitle>
          <p className="text-xs text-text-muted">Live view from connected kube contexts</p>
        </div>
        <span className="text-[11px] text-accent-primary">View all</span>
      </CardHeader>
      <DataTableShell className="rounded-none border-0">
        <DataTable className="min-w-full text-xs">
          <DataTableHeader>
            <DataTableRow>
              <DataTableHead className="px-4">Cluster</DataTableHead>
              <DataTableHead>Provider / Region</DataTableHead>
              <DataTableHead>K8s</DataTableHead>
              <DataTableHead>Nodes</DataTableHead>
              <DataTableHead>Pods</DataTableHead>
              <DataTableHead>CPU</DataTableHead>
              <DataTableHead>Memory</DataTableHead>
              <DataTableHead className="px-4">Status</DataTableHead>
            </DataTableRow>
          </DataTableHeader>
          <DataTableBody>
            {rows.map((card) => {
              const failed = card.health.pods_failed;
              const pending = card.health.pods_pending;
              const tone = !card.reachable
                ? "bad"
                : card.error ? "warn" : failed > 0
                  ? "bad"
                  : pending > 0 || card.node_ready < card.node_count
                    ? "warn"
                    : "good";
              const status = !card.reachable
                ? "Unreachable"
                : card.error ? "Incomplete" : failed > 0
                  ? "Unhealthy"
                  : pending > 0 || card.node_ready < card.node_count
                    ? "Degraded"
                    : "Healthy";
              return (
                <DataTableRow
                  key={card.context.name}
                >
                  <DataTableCell className="px-4">
                    <div className="flex min-w-[150px] items-center gap-2">
                      <span
                        className={cn(
                          "size-2 rounded-full",
                          tone === "good" && "bg-success",
                          tone === "warn" && "bg-warning",
                          tone === "bad" && "bg-danger",
                        )}
                        aria-hidden="true"
                      />
                      <div className="min-w-0">
                        <div className="truncate font-medium text-text-primary">
                          {card.context.name}
                        </div>
                        <div className="truncate text-[11px] text-text-muted">
                          {card.context.namespace ?? "all namespaces"}
                        </div>
                      </div>
                    </div>
                  </DataTableCell>
                  <DataTableCell>
                    <span className="block max-w-[220px] truncate">{card.context.cluster}</span>
                  </DataTableCell>
                  <DataTableCell className="tabular-nums">
                    <span className="inline-flex items-center gap-1.5">
                      {card.server_version ?? "—"}
                      <DistributionBadge value={card.distribution} />
                    </span>
                  </DataTableCell>
                  <DataTableCell className="tabular-nums">
                    {card.error ? "—" : `${card.node_ready}/${card.node_count}`}
                  </DataTableCell>
                  <DataTableCell className="tabular-nums">
                    {card.error ? "—" : `${card.health.pods_ready}/${card.health.pods_total}`}
                  </DataTableCell>
                  <DataTableCell>
                    <CapacityBar value={card.error ? null : card.cpu_percent} />
                  </DataTableCell>
                  <DataTableCell>
                    <CapacityBar value={card.error ? null : card.mem_percent} />
                  </DataTableCell>
                  <DataTableCell className="px-4">
                    <StatusBadge status={status} />
                  </DataTableCell>
                </DataTableRow>
              );
            })}
          </DataTableBody>
        </DataTable>
      </DataTableShell>
    </UiCard>
  );
}

function PressurePanel({ cards }: { cards: FleetCard[] }) {
  const top = [...cards]
    .filter((card) => card.reachable && !card.error)
    .sort(
      (a, b) =>
        Math.max(b.cpu_percent ?? 0, b.mem_percent ?? 0) -
        Math.max(a.cpu_percent ?? 0, a.mem_percent ?? 0),
    )
    .slice(0, 5);
  return (
    <UiCard className="p-4 shadow-none">
      <PanelHeading
        title="Top Pressure"
        meta={<Gauge className="size-4 text-accent-primary" aria-hidden="true" />}
      />
      <div className="mt-4 space-y-3">
        {top.length === 0 ? (
          <p className="text-xs text-text-muted">No connected clusters yet.</p>
        ) : (
          top.map((card) => {
            const pressure = Math.max(card.cpu_percent ?? 0, card.mem_percent ?? 0);
            return (
              <div key={card.context.name} className="space-y-1.5">
                <div className="flex items-center justify-between gap-3 text-[12px]">
                  <span className="truncate text-text-primary">{card.context.name}</span>
                  <span className="text-text-secondary tabular-nums">{Math.round(pressure)}%</span>
                </div>
                <div className="h-1.5 overflow-hidden rounded-full bg-elevated">
                  <div
                    className={cn("h-full rounded-full", heatBand(pressure))}
                    style={{ width: `${Math.min(100, Math.max(0, pressure))}%` }}
                  />
                </div>
              </div>
            );
          })
        )}
      </div>
    </UiCard>
  );
}

function AlertPanel({ cards }: { cards: FleetCard[] }) {
  const alerts = cards
    .flatMap((card) => {
      const items: Array<{ id: string; title: string; meta: string; tone: "bad" | "warn" }> = [];
      if (!card.reachable) {
        items.push({
          id: `${card.context.name}:unreachable`,
          title: "Cluster unreachable",
          meta: card.context.name,
          tone: "bad",
        });
      }
      if (card.reachable && card.error) {
        items.push({ id: `${card.context.name}:inventory`, title: "Inventory incomplete", meta: card.context.name, tone: "warn" });
        return items;
      }
      if (card.health.pods_failed > 0) {
        items.push({
          id: `${card.context.name}:failed-pods`,
          title: `${card.health.pods_failed} failed pod${card.health.pods_failed === 1 ? "" : "s"}`,
          meta: card.context.name,
          tone: "bad",
        });
      }
      if (card.health.pods_pending > 0) {
        items.push({
          id: `${card.context.name}:pending-pods`,
          title: `${card.health.pods_pending} pending pod${card.health.pods_pending === 1 ? "" : "s"}`,
          meta: card.context.name,
          tone: "warn",
        });
      }
      if ((card.cpu_percent ?? 0) >= 85 || (card.mem_percent ?? 0) >= 85) {
        items.push({
          id: `${card.context.name}:capacity`,
          title: "Capacity pressure",
          meta: card.context.name,
          tone: "warn",
        });
      }
      return items;
    })
    .slice(0, 5);
  return (
    <UiCard className="p-4 shadow-none">
      <PanelHeading
        title="Recent Signals"
        meta={<ShieldAlert className="size-4 text-warning" aria-hidden="true" />}
      />
      <div className="mt-4 space-y-2">
        {alerts.length === 0 ? (
          <p className="text-xs text-text-muted">No active signals from connected clusters.</p>
        ) : (
          alerts.map((alert) => (
            <div
              key={alert.id}
              className="flex items-start gap-2 rounded-control border border-border-subtle bg-elevated/60 p-2.5"
            >
              <AlertTriangle
                className={cn(
                  "mt-0.5 size-3.5 shrink-0",
                  alert.tone === "bad" ? "text-danger" : "text-warning",
                )}
                aria-hidden="true"
              />
              <div className="min-w-0">
                <div className="truncate text-xs font-medium text-text-primary">
                  {alert.title}
                </div>
                <div className="truncate text-[11px] text-text-muted">{alert.meta}</div>
              </div>
            </div>
          ))
        )}
      </div>
    </UiCard>
  );
}

function FleetErrorState({ error, onDiagnostics }: { error: unknown; onDiagnostics: () => void }) {
  const message = errorMessage(error);
  const isDesktopRuntimeMissing =
    message.includes("invoke") || message.includes("__TAURI_INTERNALS__");
  return (
    <div className="rounded-panel border border-danger/30 bg-[var(--status-error-soft)] p-4">
      <div className="flex items-start gap-3">
        <AlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" aria-hidden="true" />
        <div className="min-w-0">
          <h3 className="text-[13px] font-semibold text-danger">
            Unable to load cluster contexts
          </h3>
          <p className="mt-1 text-[12px] leading-5 text-danger/80">
            {isDesktopRuntimeMissing
              ? "This view needs the Tauri desktop runtime to read local kubeconfig data."
              : classifyConnectionError(message).guidance}
          </p>
          <Button className="mt-3" size="sm" variant="outline" onClick={onDiagnostics}>
            <Wrench className="size-3.5" /> Diagnose configuration
          </Button>
        </div>
      </div>
    </div>
  );
}

function HeatBar({ value, label, icon }: { value: number | null; label: string; icon: React.ReactNode }) {
  return (
    <div className="flex items-center gap-2 min-w-0">
      <span className="text-term-subtle shrink-0">{icon}</span>
      <span className="text-[11px] text-term-muted shrink-0 w-7">{label}</span>
      <div className="flex-1 h-1.5 bg-term-panel-2 rounded-full overflow-hidden min-w-[40px]">
        <div
          className={cn("h-full transition-all", heatBand(value))}
          style={{ width: `${value === null ? 0 : Math.min(100, Math.max(0, value))}%` }}
        />
      </div>
      <span className="text-[11px] text-term-fg w-10 text-right tabular-nums">{pct(value)}</span>
    </div>
  );
}

function PodRatioRing({ card }: { card: FleetCard }) {
  const { pods_total, pods_ready, pods_pending, pods_failed } = card.health;
  const total = Math.max(1, pods_total);
  const readyPct = (pods_ready / total) * 100;
  const pendingPct = (pods_pending / total) * 100;
  const failedPct = (pods_failed / total) * 100;
  const r = 22;
  const circ = 2 * Math.PI * r;
  const seg = (p: number) => (p / 100) * circ;
  return (
    <div className="relative w-[68px] h-[68px] shrink-0">
      <svg viewBox="0 0 60 60" className="w-full h-full -rotate-90">
        <circle cx="30" cy="30" r={r} stroke="var(--term-panel-2)" strokeWidth="6" fill="none" />
        <circle
          cx="30"
          cy="30"
          r={r}
          stroke="var(--term-red)"
          strokeWidth="6"
          fill="none"
          strokeDasharray={`${seg(failedPct)} ${circ}`}
          strokeDashoffset={0}
          strokeLinecap="round"
        />
        <circle
          cx="30"
          cy="30"
          r={r}
          stroke="var(--status-warning)"
          strokeWidth="6"
          fill="none"
          strokeDasharray={`${seg(pendingPct)} ${circ}`}
          strokeDashoffset={-seg(failedPct)}
          strokeLinecap="round"
        />
        <circle
          cx="30"
          cy="30"
          r={r}
          stroke="var(--status-success)"
          strokeWidth="6"
          fill="none"
          strokeDasharray={`${seg(readyPct)} ${circ}`}
          strokeDashoffset={-(seg(failedPct) + seg(pendingPct))}
          strokeLinecap="round"
        />
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center text-center">
        <span className="text-[13px] font-semibold text-term-fg tabular-nums leading-none">
          {pods_ready}
        </span>
        <span className="text-[9px] text-term-subtle tabular-nums">/{pods_total}</span>
      </div>
    </div>
  );
}

function Card({
  entry,
  labels,
  selected,
  onOpen,
  onConnect,
  onDisconnect,
  onDiagnose,
  onDelete,
  onAddLabel,
  onRemoveLabel,
}: {
  entry: FleetEntry;
  labels: string[];
  selected: boolean;
  onOpen: () => void;
  onConnect: () => void;
  onDisconnect: () => void;
  onDiagnose: () => void;
  onDelete: () => void;
  onAddLabel: (label: string) => void;
  onRemoveLabel: (label: string) => void;
}) {
  const { context, card, connecting } = entry;
  if (!card) {
    return (
      <article
        data-testid="fleet-card"
        className={cn(
          "flex min-h-[214px] flex-col justify-between gap-3 rounded-panel border bg-surface p-4 text-left transition-colors hover:border-accent-primary/35 hover:bg-elevated",
          selected ? "border-accent-primary/60 ring-1 ring-accent-primary/25" : "border-border-default",
        )}
      >
        <div className="flex items-start gap-3">
          <div className="size-2 rounded-full mt-1.5 shrink-0 bg-term-subtle" />
          <div className="flex-1 min-w-0">
            <button
              type="button"
              onClick={onOpen}
              className={cn(
                "block max-w-full text-[14px] font-semibold text-term-fg truncate text-left",
                "hover:text-term-green focus:outline-none focus-visible:ring-2 focus-visible:ring-term-green/60 rounded-sm",
              )}
            >
              {context.name}
            </button>
            <div className="text-[11px] text-term-subtle truncate mt-0.5">
              {context.cluster} · {context.user}
            </div>
          </div>
        </div>

        <div className="rounded-control border border-border-default bg-elevated p-3">
          <div className="text-[10px] uppercase tracking-wider text-term-subtle">status</div>
          <div className="mt-1 text-[13px] text-term-fg">
            {connecting ? "connecting..." : "not connected"}
          </div>
          <div className="mt-1 text-[11px] text-term-muted">live metrics paused</div>
        </div>

        <ClusterLabels
          contextName={context.name}
          labels={labels}
          onAdd={onAddLabel}
          onRemove={onRemoveLabel}
        />

        <div className="grid grid-cols-[1fr_auto] gap-3 pt-2 border-t border-term-border-soft">
          <div data-testid="fleet-card-workflow-actions" className="flex items-center">
            <button
              type="button"
              onClick={onConnect}
              disabled={connecting}
              aria-label={`connect ${context.name}`}
              className="term-btn !min-h-[30px] !py-1 !px-3 !text-[12px]"
            >
              <RefreshCw className={cn("size-3.5", connecting && "animate-spin")} />
              {connecting ? "connecting" : "connect"}
            </button>
          </div>
          <div
            data-testid="fleet-card-danger-actions"
            className="flex items-center border-l border-term-border-soft pl-3"
          >
            <button
              type="button"
              onClick={onDelete}
              aria-label={`delete ${context.name}`}
              className="term-btn !min-h-[30px] !py-1 !px-3 !text-[12px] text-term-muted hover:border-term-red/50 hover:text-term-red"
            >
              delete
            </button>
          </div>
        </div>
      </article>
    );
  }

  const unreachable = !card.reachable;
  const isProd = card.context.is_prod;
  return (
    <article
      data-testid="fleet-card"
      className={cn(
        "text-left group relative flex flex-col gap-3 p-4 rounded-panel border transition-colors min-h-[214px] justify-between",
        selected
          ? "border-accent-primary/60 bg-surface ring-1 ring-accent-primary/25"
          : "border-border-default bg-surface",
        "hover:border-accent-primary/35 hover:bg-elevated",
        unreachable && "opacity-75 hover:border-term-border-soft",
      )}
    >
      <div className="flex items-start gap-3">
        <div
          className={cn(
            "size-2 rounded-full mt-1.5 shrink-0",
            unreachable ? "bg-term-red" : card.error ? "bg-warning" : "bg-success animate-pulse",
          )}
        />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <button
              type="button"
              onClick={onOpen}
              disabled={unreachable}
              className={cn(
                "text-[14px] font-semibold text-term-fg truncate text-left",
                "focus:outline-none focus-visible:ring-2 focus-visible:ring-term-green/60 rounded-sm",
                unreachable && "cursor-not-allowed",
              )}
            >
              {card.context.name}
            </button>
            {isProd && (
          <span className="rounded border border-danger/40 bg-[var(--status-error-soft)] px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-danger">
                prod
              </span>
            )}
            {card.server_version && (
              <span className="text-[11px] text-term-subtle tabular-nums">
                {card.server_version}
              </span>
            )}
          </div>
          <div className="text-[11px] text-term-subtle truncate mt-0.5">
            {card.context.cluster} · {card.context.user}
          </div>
        </div>
        {!unreachable && !card.error && <PodRatioRing card={card} />}
      </div>

      {unreachable ? (
        <div className="flex items-start gap-2 p-2 rounded bg-term-red/10 border border-term-red/30 text-[12px] text-term-red">
          <AlertTriangle className="size-3.5 mt-0.5 shrink-0" aria-hidden="true" />
          <span className="truncate">{classifyConnectionError(card.error ?? "unreachable").title}</span>
        </div>
      ) : card.error ? (
        <p role="status" className="text-xs text-warning">{card.error}</p>
      ) : (
        <>
          <div className="grid grid-cols-3 gap-3 text-[12px]">
            <div className="flex flex-col">
              <span className="text-term-subtle text-[10px] uppercase tracking-wider">nodes</span>
              <span className="text-term-fg tabular-nums">
                <span className={card.node_ready < card.node_count ? "text-warning" : ""}>
                  {card.node_ready}
                </span>
                <span className="text-term-subtle">/{card.node_count}</span>
              </span>
            </div>
            <div className="flex flex-col">
              <span className="text-term-subtle text-[10px] uppercase tracking-wider">ns</span>
              <span className="text-term-fg tabular-nums">{card.namespace_count}</span>
            </div>
            <div className="flex flex-col">
              <span className="text-term-subtle text-[10px] uppercase tracking-wider">workloads</span>
              <span className="text-term-fg tabular-nums">{card.workload_count}</span>
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <HeatBar value={card.cpu_percent} label="cpu" icon={<Cpu className="size-3" />} />
            <HeatBar value={card.mem_percent} label="mem" icon={<MemoryStick className="size-3" />} />
          </div>
        </>
      )}

      <ClusterLabels
        contextName={context.name}
        labels={labels}
        onAdd={onAddLabel}
        onRemove={onRemoveLabel}
      />

      <div className="pt-2 border-t border-term-border-soft text-[11px] text-term-subtle">
        <div className="flex items-center gap-1">
          <CircleDot className="size-3" />
          <span className="tabular-nums">
            {new Date(card.fetched_at_ms).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
              second: "2-digit",
            })}
          </span>
        </div>
        <div className="mt-2 grid grid-cols-[1fr_auto_auto] gap-3 items-center">
          <div data-testid="fleet-card-workflow-actions" className="flex items-center">
            {unreachable ? (
              <span className="text-[11px] text-term-muted">triage unavailable</span>
            ) : (
              <button
                type="button"
                onClick={onOpen}
                aria-label={`triage ${context.name}`}
                className={cn(
                  "term-btn !min-h-[30px] !py-1 !px-4 !text-[12px]",
                  // Triage CTA — violet is the only accent in this slot,
                  // so paired light/dark variants instead of a new semantic.
                  "border-violet-700/50 bg-violet-100 text-violet-900",
                  "dark:border-violet-400/40 dark:bg-violet-500/15 dark:text-violet-100",
                  "hover:border-violet-700/80 hover:bg-violet-200",
                  "dark:hover:border-violet-300/70 dark:hover:bg-violet-500/25 dark:hover:text-white",
                  "focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-700/60 dark:focus-visible:ring-violet-300/60",
                )}
              >
                triage
              </button>
            )}
          </div>
          <div data-testid="fleet-card-connection-actions" className="flex items-center">
            {unreachable ? (
              <div className="flex gap-2">
                <button type="button" onClick={onDiagnose} aria-label={`diagnose ${context.name}`} className="term-btn !min-h-[28px] !py-1 !px-2 !text-[11px]">
                  <Wrench className="size-3" /> diagnose
                </button>
                <button
                  type="button"
                  onClick={onConnect}
                  disabled={connecting}
                  aria-label={`connect ${context.name}`}
                  className="term-btn !min-h-[28px] !py-1 !px-2 !text-[11px]"
                >
                  <RefreshCw className={cn("size-3", connecting && "animate-spin")} />
                  retry
                </button>
              </div>
            ) : (
              <button
                type="button"
                onClick={onDisconnect}
                aria-label={`disconnect ${context.name}`}
                className="term-btn !min-h-[28px] !py-1 !px-2 !text-[11px]"
              >
                disconnect
              </button>
            )}
          </div>
          <div
            data-testid="fleet-card-danger-actions"
            className="flex items-center border-l border-term-border-soft pl-3"
          >
            <button
              type="button"
              onClick={onDelete}
              aria-label={`delete ${context.name}`}
              className="term-btn !min-h-[28px] !py-1 !px-2 !text-[11px] text-term-muted hover:border-term-red/50 hover:text-term-red"
            >
              delete
            </button>
          </div>
        </div>
      </div>
    </article>
  );
}

const LABEL_TONES = [
  "border-success/35 bg-success/10 text-success",
  "border-info/35 bg-info/10 text-info",
  "border-warning/35 bg-warning/10 text-warning",
  "border-accent-primary/35 bg-accent-primary-soft text-accent-primary",
];

function labelTone(label: string): string {
  let hash = 0;
  for (const char of label) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return LABEL_TONES[hash % LABEL_TONES.length];
}

function ClusterLabels({
  contextName,
  labels,
  onAdd,
  onRemove,
}: {
  contextName: string;
  labels: string[];
  onAdd: (label: string) => void;
  onRemove: (label: string) => void;
}) {
  const [draft, setDraft] = useState("");

  function submit() {
    const value = draft.trim();
    if (!value) return;
    onAdd(value);
    setDraft("");
  }

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {labels.map((label) => (
        <span
          key={label}
          className={cn(
            "inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] font-medium",
            labelTone(label),
          )}
        >
          <Tag className="size-2.5" aria-hidden="true" />
          {label}
          <button
            type="button"
            onClick={() => onRemove(label)}
            aria-label={`remove ${label} label from ${contextName}`}
            className="text-term-subtle hover:text-term-red"
          >
            <X className="size-2.5" aria-hidden="true" />
          </button>
        </span>
      ))}
      <div className="inline-flex items-center gap-1 rounded border border-border-default bg-elevated px-1.5 py-0.5">
        <input
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === ",") {
              event.preventDefault();
              submit();
            }
          }}
          aria-label={`add label to ${contextName}`}
          placeholder="label"
          className="w-[64px] bg-transparent text-[10px] text-term-fg placeholder:text-term-subtle focus:outline-none"
        />
        <button
          type="button"
          onClick={submit}
          aria-label={`save label for ${contextName}`}
          className="text-term-subtle hover:text-term-green"
        >
          <Plus className="size-3" aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

export function FleetView() {
  const nav = useNavigate();
  const { contextName: activeContextName, setContext } = useClusterStore();
  const [cardsByContext, setCardsByContext] = useState<Record<string, FleetCard>>(
    readStoredCards,
  );
  const [connecting, setConnecting] = useState<Set<string>>(() => new Set());
  const [hidden, setHidden] = useState<Set<string>>(readStoredHidden);
  const [labelsByContext, setLabelsByContext] = useState<Record<string, string[]>>(
    readStoredLabels,
  );
  const [trashOpen, setTrashOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ContextInfo | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<DeletedContextSummary | null>(null);
  const [restoreBusy, setRestoreBusy] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [diagnosticsContext, setDiagnosticsContext] = useState<string | null>(null);
  const [probeErrorsByContext, setProbeErrorsByContext] = useState<Record<string, unknown>>({});
  const { data: contexts = [], isLoading, isFetching, refetch, error } = useQuery({
    queryKey: ["k8s", "contexts"],
    queryFn: k8s.listContexts,
    staleTime: 5_000,
  });
  const {
    data: deletedContexts = [],
    isFetching: isTrashFetching,
    refetch: refetchDeletedContexts,
  } = useQuery({
    queryKey: ["k8s", "deleted-contexts"],
    queryFn: k8s.listDeletedContexts,
    staleTime: 5_000,
  });

  const sorted = useMemo(() => {
    return contexts
      .filter((context) => !hidden.has(context.name))
      .map((context) => ({
        context,
        card: cardsByContext[context.name],
        connecting: connecting.has(context.name),
      }))
      .sort((a, b) => {
        const risk = entryRiskScore(a) - entryRiskScore(b);
        if (risk !== 0) return risk;
        if (a.context.is_prod !== b.context.is_prod) return a.context.is_prod ? -1 : 1;
        return a.context.name.localeCompare(b.context.name);
      });
  }, [cardsByContext, connecting, contexts, hidden]);

  const cards = sorted.flatMap((entry) => (entry.card ? [entry.card] : []));
  const reach = cards.filter((c) => c.reachable && !c.error);
  const totalNodes = reach.reduce((sum, card) => sum + card.node_count, 0);
  const readyNodes = reach.reduce((sum, card) => sum + card.node_ready, 0);
  const totalPods = reach.reduce((sum, card) => sum + card.health.pods_total, 0);
  const unhealthyPods = reach.reduce(
    (sum, card) => sum + card.health.pods_failed + card.health.pods_pending,
    0,
  );
  const totalWorkloads = reach.reduce((sum, card) => sum + card.workload_count, 0);
  const avgCpu = average(reach.map((card) => card.cpu_percent));
  const avgMem = average(reach.map((card) => card.mem_percent));
  const riskTotals = useMemo(
    () => ({
      connected: cards.length,
      disconnected: sorted.filter((entry) => !entry.card).length,
      unreachable: cards.filter((c) => !c.reachable).length,
      prodAlerts: reach.filter((c) => c.context.is_prod && c.health.pods_failed > 0).length,
      pressure: reach.filter(
        (c) => (c.cpu_percent ?? 0) >= 90 || (c.mem_percent ?? 0) >= 90,
      ).length,
    }),
    [cards, reach, sorted],
  );

  useEffect(() => {
    writeStoredCards(cardsByContext);
  }, [cardsByContext]);

  useEffect(() => {
    writeStoredHidden(hidden);
  }, [hidden]);

  useEffect(() => {
    writeStoredLabels(labelsByContext);
  }, [labelsByContext]);

  function addLabel(contextName: string, label: string) {
    const normalized = label.trim().replace(/\s+/g, "-").toLowerCase();
    if (!normalized) return;
    setLabelsByContext((prev) => {
      const existing = prev[contextName] ?? [];
      if (existing.includes(normalized)) return prev;
      return { ...prev, [contextName]: [...existing, normalized] };
    });
  }

  function removeLabel(contextName: string, label: string) {
    setLabelsByContext((prev) => {
      const nextLabels = (prev[contextName] ?? []).filter((value) => value !== label);
      const next = { ...prev };
      if (nextLabels.length === 0) {
        delete next[contextName];
      } else {
        next[contextName] = nextLabels;
      }
      return next;
    });
  }

  async function connectContext(contextName: string) {
    setConnecting((prev) => new Set(prev).add(contextName));
    try {
      const card = await k8s.probeFleetContext(contextName);
      setProbeErrorsByContext((prev) => {
        const next = { ...prev };
        delete next[contextName];
        return next;
      });
      setContext(card.context.name);
      setCardsByContext((prev) => ({ ...prev, [contextName]: card }));
      await k8s.setContext(contextName).catch(() => undefined);
    } catch (err) {
      setProbeErrorsByContext((prev) => ({ ...prev, [contextName]: err }));
      setCardsByContext((prev) => {
        const existing = prev[contextName];
        if (!existing) return prev;
        return {
          ...prev,
          [contextName]: {
            ...existing,
            reachable: false,
            error: errorMessage(err),
            fetched_at_ms: Date.now(),
          },
        };
      });
    } finally {
      setConnecting((prev) => {
        const next = new Set(prev);
        next.delete(contextName);
        return next;
      });
    }
  }

  function openDiagnostics(contextName: string | null) {
    setDiagnosticsContext(contextName);
    setDiagnosticsOpen(true);
  }

  async function disconnectContext(contextName: string) {
    await k8s.disconnectContext(contextName);
    setCardsByContext((prev) => {
      const next = { ...prev };
      delete next[contextName];
      return next;
    });
  }

  async function deleteContext(contextName: string) {
    setDeleteBusy(true);
    setDeleteError(null);
    try {
      await k8s.deleteContext(contextName);
      setHidden((prev) => new Set(prev).add(contextName));
      setCardsByContext((prev) => {
        const next = { ...prev };
        delete next[contextName];
        return next;
      });
      await refetchDeletedContexts();
      setDeleteTarget(null);
    } catch (err) {
      setDeleteError(errorMessage(err));
    } finally {
      setDeleteBusy(false);
    }
  }

  async function restoreContext(contextName: string, overwrite = false) {
    setRestoreBusy(true);
    setRestoreError(null);
    try {
      await k8s.restoreDeletedContext(contextName, overwrite);
      setHidden((prev) => {
        const next = new Set(prev);
        next.delete(contextName);
        return next;
      });
      await Promise.all([refetch(), refetchDeletedContexts()]);
      setRestoreTarget(null);
    } catch (err) {
      setRestoreError(errorMessage(err));
      if (!overwrite) {
        const target = deletedContexts.find((context) => context.name === contextName);
        if (target) setRestoreTarget({ ...target, has_conflict: true });
      }
    } finally {
      setRestoreBusy(false);
    }
  }

  async function rescan() {
    setHidden(new Set());
    await Promise.all([refetch(), refetchDeletedContexts()]);
  }

  return (
    <LumenPage className="gap-5">
      <PageHeader
        eyebrow="Global dashboard"
        title="Multi-cluster overview"
        icon={<Cloud className="size-3.5" aria-hidden="true" />}
        description={
          <>
            {sorted.length} context{sorted.length === 1 ? "" : "s"} discovered · {cards.length}{" "}
            connected · {reach.length} reporting live data
          </>
        }
        actions={
          <div className="flex flex-wrap items-center gap-2">
            <div className="hidden min-w-[260px] items-center gap-2 rounded-control border border-border-default bg-elevated px-3 py-2 text-xs text-text-muted md:flex">
              <Database className="size-3.5" aria-hidden="true" />
              <span className="truncate">Local kubeconfig · no cloud account required</span>
            </div>
            <Button
              onClick={rescan}
              disabled={isFetching}
              title="Scan local kubeconfig contexts and restore removed clusters."
            >
              <RefreshCw className={cn("size-3.5", isFetching && "animate-spin")} />
              Rescan
            </Button>
          </div>
        }
      />

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-6">
            <MetricTile
              icon={<Cloud className="size-3.5" />}
              label="Clusters"
              value={formatNumber(sorted.length)}
              sub={
                <span>
                  {riskTotals.unreachable === 0 ? "All reachable" : `${riskTotals.unreachable} unreachable`}
                </span>
              }
              tone={riskTotals.unreachable > 0 ? "bad" : "good"}
            />
            <MetricTile
              icon={<Server className="size-3.5" />}
              label="Nodes"
              value={reach.length ? formatNumber(totalNodes) : "—"}
              sub={
                <span>
                  {reach.length ? `${readyNodes}/${totalNodes} ready` : "Connect to scan"}
                </span>
              }
              tone={readyNodes < totalNodes ? "warn" : "good"}
            />
            <MetricTile
              icon={<CircleDot className="size-3.5" />}
              label="Pods"
              value={reach.length ? formatNumber(totalPods) : "—"}
              sub={
                <span>
                  {reach.length ? `${unhealthyPods} unhealthy` : "Waiting for data"}
                </span>
              }
              tone={unhealthyPods > 0 ? "bad" : "good"}
            />
            <MetricTile
              icon={<Layers3 className="size-3.5" />}
              label="Workloads"
              value={reach.length ? formatNumber(totalWorkloads) : "—"}
              sub={<span>{reach.length ? "Across namespaces" : "Connect clusters"}</span>}
              tone="info"
            />
            <MetricTile
              icon={<Cpu className="size-3.5" />}
              label="CPU Usage"
              value={avgCpu === null ? "—" : `${Math.round(avgCpu)}%`}
              sub={<span>{avgCpu === null ? "Metrics unavailable" : "Fleet average"}</span>}
              tone={avgCpu !== null && avgCpu >= 85 ? "warn" : "info"}
              trend={avgCpu !== null}
            />
            <MetricTile
              icon={<MemoryStick className="size-3.5" />}
              label="Memory"
              value={avgMem === null ? "—" : `${Math.round(avgMem)}%`}
              sub={<span>{avgMem === null ? "Metrics unavailable" : "Fleet average"}</span>}
              tone={avgMem !== null && avgMem >= 85 ? "warn" : "info"}
              trend={avgMem !== null}
            />
          </div>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1fr)_340px]">
            <ClusterHealthTable cards={cards} />
            <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-1">
              <PressurePanel cards={cards} />
              <AlertPanel cards={cards} />
            </div>
          </div>

          <SectionPanel>
            <PanelHeading
              eyebrow="Operations"
              title="Cluster workflow cards"
              icon={<Bot className="size-3.5" aria-hidden="true" />}
              meta={
                <div className="flex items-center gap-3">
                <span>{riskTotals.disconnected} offline</span>
                  <span className="h-3 w-px bg-border-default" aria-hidden="true" />
                <span>{riskTotals.pressure} under pressure</span>
              </div>
              }
            />
            {error ? (
              <FleetErrorState error={error} onDiagnostics={() => openDiagnostics(null)} />
            ) : isLoading ? (
              <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
                {Array.from({ length: 6 }).map((_, i) => (
                  <div
                    key={i}
                    className="h-[200px] animate-pulse rounded-panel border border-border-default bg-surface"
                  />
                ))}
              </div>
            ) : (
              <>
                {sorted.length === 0 ? (
                  <div className="rounded-control border border-border-default bg-elevated p-4 text-sm text-text-secondary">
                    <p>No active contexts were found in the configured kubeconfig sources.</p>
                    <Button className="mt-3" size="sm" variant="outline" onClick={() => openDiagnostics(null)}>
                      <Wrench className="size-3.5" /> Diagnose kubeconfig
                    </Button>
                  </div>
                ) : (
                  <div
                    data-testid="fleet-card-grid"
                    className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3"
                  >
                    {sorted.map((entry) => (
                      <Card
                        key={entry.context.name}
                        entry={entry}
                        labels={labelsByContext[entry.context.name] ?? []}
                        selected={activeContextName === entry.context.name}
                        onOpen={() =>
                          nav(`/cluster/${encodeURIComponent(entry.context.name)}/workloads`)
                        }
                        onConnect={() => connectContext(entry.context.name)}
                        onDisconnect={() => disconnectContext(entry.context.name)}
                        onDiagnose={() => openDiagnostics(entry.context.name)}
                        onDelete={() => {
                          setDeleteError(null);
                          setDeleteTarget(entry.context);
                        }}
                        onAddLabel={(label) => addLabel(entry.context.name, label)}
                        onRemoveLabel={(label) => removeLabel(entry.context.name, label)}
                      />
                    ))}
                  </div>
                )}
                <TrashSection
                  contexts={deletedContexts}
                  busy={isTrashFetching || restoreBusy}
                  open={trashOpen}
                  onOpenChange={setTrashOpen}
                  onRestore={(context) => {
                    setRestoreError(null);
                    if (context.has_conflict) {
                      setRestoreTarget(context);
                    } else {
                      void restoreContext(context.name);
                    }
                  }}
                />
              </>
            )}
          </SectionPanel>
      {deleteTarget && (
        <DeleteContextDialog
          context={deleteTarget}
          busy={deleteBusy}
          error={deleteError}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => void deleteContext(deleteTarget.name)}
        />
      )}
      {restoreTarget && (
        <RestoreContextDialog
          context={restoreTarget}
          busy={restoreBusy}
          error={restoreError}
          onCancel={() => setRestoreTarget(null)}
          onRestore={() => void restoreContext(restoreTarget.name, true)}
        />
      )}
      <ConnectionDiagnosticsDialog
        open={diagnosticsOpen}
        context={diagnosticsContext}
        observedError={
          diagnosticsContext
            ? probeErrorsByContext[diagnosticsContext] ??
              cardsByContext[diagnosticsContext]?.error ??
              undefined
            : error
        }
        retrying={diagnosticsContext ? connecting.has(diagnosticsContext) : false}
        onClose={() => setDiagnosticsOpen(false)}
        onRetry={() => {
          if (diagnosticsContext) return connectContext(diagnosticsContext);
          return rescan();
        }}
      />
    </LumenPage>
  );
}

function TrashSection({
  contexts,
  busy,
  open,
  onOpenChange,
  onRestore,
}: {
  contexts: DeletedContextSummary[];
  busy: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRestore: (context: DeletedContextSummary) => void;
}) {
  if (contexts.length === 0) return null;
  return (
    <section className="mt-6 border-t border-term-border-soft pt-4">
      <button
        type="button"
        onClick={() => onOpenChange(!open)}
        aria-expanded={open}
        className="flex w-full items-center justify-between gap-3 rounded-md px-1 py-1 text-left hover:bg-term-panel/60 focus:outline-none focus-visible:ring-2 focus-visible:ring-term-green/60"
      >
        <div className="flex items-center gap-2">
          <Trash2 className="size-4 text-term-subtle" />
          <h2 className="text-[13px] font-semibold text-term-fg">trash</h2>
          <span className="text-[11px] text-term-muted">
            {contexts.length} deleted · kept for 90 days
          </span>
        </div>
        <ChevronDown
          className={cn("size-4 text-term-subtle transition-transform", open && "rotate-180")}
          aria-hidden="true"
        />
      </button>
      {open && <div className="mt-3 grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {contexts.map((context) => (
          <article
            key={context.name}
            className="rounded-panel border border-border-default bg-surface p-3"
          >
            <div className="flex items-start gap-2">
              <div className="size-2 rounded-full mt-1.5 bg-term-muted shrink-0" />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <div className="truncate text-[13px] font-semibold text-term-fg">
                    {context.name}
                  </div>
                  {context.is_prod && (
                    <span className="px-1.5 py-0.5 text-[9px] rounded bg-term-red/20 text-term-red border border-term-red/40 font-semibold uppercase tracking-wide">
                      prod
                    </span>
                  )}
                </div>
                <div className="mt-0.5 truncate text-[11px] text-term-muted">
                  {context.cluster} · {context.user}
                </div>
              </div>
            </div>
            <div className="mt-3 flex items-center justify-between gap-2 border-t border-term-border-soft pt-3">
              <div className="text-[11px] text-term-subtle">
                expires in {context.days_remaining}d
                {context.has_conflict && (
                  <span className="ml-2 text-warning">conflict</span>
                )}
              </div>
              <button
                type="button"
                onClick={() => onRestore(context)}
                disabled={busy}
                aria-label={`restore ${context.name}`}
                className="term-btn !min-h-[28px] !py-1 !px-2 !text-[11px]"
              >
                <ArchiveRestore className="size-3" />
                restore
              </button>
            </div>
          </article>
        ))}
      </div>}
    </section>
  );
}

function DeleteContextDialog({
  context,
  busy,
  error,
  onCancel,
  onConfirm,
}: {
  context: ContextInfo;
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 px-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="delete-context-title"
        className="w-full max-w-[420px] rounded-panel border border-border-default bg-surface shadow-[var(--shadow-popover)]"
      >
        <div className="p-4 border-b border-term-border-soft">
          <h2 id="delete-context-title" className="text-[15px] font-semibold text-term-fg">
            delete cluster from fleet?
          </h2>
          <p className="mt-1 text-[12px] text-term-muted">
            This moves the context to trash for 90 days, then removes it from your local kubeconfig.
            You can restore it from trash unless the backup expires.
          </p>
        </div>
        <div className="p-4 space-y-3">
          <div className="rounded border border-term-border-soft bg-term-bg/40 p-3">
            <div className="text-[10px] uppercase tracking-wider text-term-subtle">target</div>
            <div className="mt-1 text-[13px] text-term-fg font-mono break-all">{context.name}</div>
            <div className="mt-1 text-[11px] text-term-muted break-all">
              {context.cluster} · {context.user}
            </div>
          </div>
          {error && (
            <div className="rounded border border-term-red/40 bg-term-red/10 p-2 text-[12px] text-term-red">
              {error}
            </div>
          )}
          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={onCancel}
              disabled={busy}
              className="term-btn !min-h-[32px] !py-1.5 !px-3 !text-[12px]"
            >
              cancel
            </button>
            <button
              type="button"
              onClick={onConfirm}
              disabled={busy}
              className="term-btn !min-h-[32px] !py-1.5 !px-3 !text-[12px] text-term-red hover:border-term-red/60"
            >
              {busy ? "deleting..." : "delete"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function RestoreContextDialog({
  context,
  busy,
  error,
  onCancel,
  onRestore,
}: {
  context: DeletedContextSummary;
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onRestore: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 px-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="restore-context-title"
        className="w-full max-w-[440px] rounded-panel border border-border-default bg-surface shadow-[var(--shadow-popover)]"
      >
        <div className="p-4 border-b border-term-border-soft">
          <h2 id="restore-context-title" className="text-[15px] font-semibold text-term-fg">
            restore deleted cluster?
          </h2>
          <p className="mt-1 text-[12px] text-term-muted">
            A context, cluster, or user with the same name is already present. Restore can override
            the current kubeconfig entries with the trash backup.
          </p>
        </div>
        <div className="p-4 space-y-3">
          <div className="rounded border border-term-border-soft bg-term-bg/40 p-3">
            <div className="text-[10px] uppercase tracking-wider text-term-subtle">backup</div>
            <div className="mt-1 text-[13px] text-term-fg font-mono break-all">{context.name}</div>
            <div className="mt-1 text-[11px] text-term-muted break-all">
              {context.cluster} · {context.user}
            </div>
          </div>
          {error && (
            <div className="rounded border border-term-red/40 bg-term-red/10 p-2 text-[12px] text-term-red">
              {error}
            </div>
          )}
          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={onCancel}
              disabled={busy}
              className="term-btn !min-h-[32px] !py-1.5 !px-3 !text-[12px]"
            >
              keep current
            </button>
            <button
              type="button"
              onClick={onRestore}
              disabled={busy}
              className="term-btn !min-h-[32px] !py-1.5 !px-3 !text-[12px] text-warning hover:border-warning/60"
            >
              {busy ? "restoring..." : "override and restore"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
