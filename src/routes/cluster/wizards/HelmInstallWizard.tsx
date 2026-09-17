import { HelmRepositories } from "@/components/HelmRepositories";
import { useMutationCapability } from "@/hooks/useMutationCapability";
import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft,
  CheckCircle2,
  Lock,
  Package,
  Search,
  Sparkles,
} from "lucide-react";
import { toast } from "sonner";
import { k8s } from "@/lib/k8s";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { LumenPage, PageHeader, SectionPanel } from "@/components/lumen/page";
import { HelmActionDialog, type HelmDialogAction } from "@/components/HelmActionDialog";
import { PreflightPreviewDialog } from "@/components/PreflightPreviewDialog";
import { buildActionPreflight } from "@/lib/preflight";
import {
  buildPreviewCommand,
  groupSearchHits,
  toInstallRequest,
  toUpgradeRequest,
  validateInstall,
  validateUpgrade,
  type HelmWizardSpec,
} from "@/lib/wizards/helm";

type WizardMode = "install" | "upgrade";

/**
 * Helm install / upgrade wizard.
 *
 * The same component handles both modes — `mode === "upgrade"` pre-fills
 * release / chart / namespace from the existing release detail and disables
 * fields that are not safe to change (release name, namespace, create-ns).
 *
 * The "review" pane shows a copy-pasteable `helm` command line so users can
 * see what's actually going to run before they hit execute. dry-run streams
 * the rendered manifest into a scrollable pane via the same Channel that
 * actual installs use — sharing the dialog keeps the UX consistent.
 */
export function HelmInstallWizard() {
  return <HelmWizard mode="install" />;
}

export function HelmUpgradeWizard() {
  return <HelmWizard mode="upgrade" />;
}

function HelmWizard({ mode }: { mode: WizardMode }) {
  const { ctx = "", release: releaseParam = "" } = useParams();
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const context = decodeURIComponent(ctx);
  const releaseName = decodeURIComponent(releaseParam);
  const initialNamespace = searchParams.get("ns") ?? "";
  const capability = useMutationCapability(context);
  const readOnly = !capability.canMutate;

  const namespaces = useQuery({
    queryKey: ["k8s", "namespaces", context],
    queryFn: () => k8s.listNamespaces(context || undefined),
    staleTime: 60_000,
  });

  // For upgrade mode, load the existing release so we can pre-fill chart,
  // version, and current values.
  const existing = useQuery({
    queryKey: ["k8s", "helm-detail", context, initialNamespace, releaseName, null],
    queryFn: () =>
      k8s.getHelmRelease(initialNamespace, releaseName, undefined, context || undefined),
    enabled: mode === "upgrade" && !!initialNamespace && !!releaseName,
    staleTime: 30_000,
  });

  const [spec, setSpec] = useState<HelmWizardSpec>({
    chart: "",
    version: "",
    release: mode === "upgrade" ? releaseName : "",
    namespace: initialNamespace || "default",
    valuesYaml: "",
    createNamespace: false,
    atomic: true,
    wait: false,
  });
  const [pendingAction, setPendingAction] = useState<HelmDialogAction | null>(null);
  const [pendingPreviewAction, setPendingPreviewAction] = useState<HelmDialogAction | null>(null);
  const [dryRunPending, setDryRunPending] = useState<HelmDialogAction | null>(null);
  const [valuesPrefilled, setValuesPrefilled] = useState(mode === "upgrade");

  // Pre-fill from existing release when upgrade detail arrives.
  useEffect(() => {
    if (mode !== "upgrade" || !existing.data) return;
    const det = existing.data;
    setSpec((prev) => ({
      ...prev,
      chart: prev.chart || det.summary.chart_name,
      version: prev.version || det.summary.chart_version,
      namespace: prev.namespace === "default" ? det.summary.namespace : prev.namespace,
      valuesYaml: prev.valuesYaml || (det.user_values_yaml === "{}" ? "" : det.user_values_yaml),
    }));
  }, [mode, existing.data]);

  const errors = useMemo(
    () => (mode === "install" ? validateInstall(spec) : validateUpgrade(spec)),
    [mode, spec],
  );

  function startRun(dryRun: boolean) {
    if (errors.length > 0) {
      toast.error(errors.join("; "));
      return;
    }
    if (readOnly && !dryRun) {
      toast.error(capability.reason);
      return;
    }
    if (mode === "install") {
      const action: HelmDialogAction = {
        kind: "install",
        request: { ...toInstallRequest(spec), dry_run: dryRun },
      };
      if (dryRun) setDryRunPending(action);
      else setPendingPreviewAction(action);
    } else {
      const action: HelmDialogAction = {
        kind: "upgrade",
        request: { ...toUpgradeRequest(spec), dry_run: dryRun },
      };
      if (dryRun) setDryRunPending(action);
      else setPendingPreviewAction(action);
    }
  }

  const previewCmd = useMemo(
    () =>
      buildPreviewCommand({
        mode,
        context: context || undefined,
        spec,
        dryRun: false,
      }),
    [mode, context, spec],
  );
  const helmPreflight = useMemo(() => {
    if (!pendingPreviewAction) return null;
    if (pendingPreviewAction.kind !== "install" && pendingPreviewAction.kind !== "upgrade") {
      return null;
    }
    const request = pendingPreviewAction.request;
    return buildActionPreflight({
      actionType: pendingPreviewAction.kind === "install" ? "helm-install" : "helm-upgrade",
      targets: [
        {
          kind: "helmrelease",
          namespace: request.namespace || null,
          name: request.release,
        },
      ],
      note: `Chart ${request.chart}${request.version ? `@${request.version}` : ""} will run through your local helm CLI.`,
    });
  }, [pendingPreviewAction]);

  return (
    <LumenPage>
      <PageHeader
        eyebrow="wizards"
        title={mode === "install" ? "Install Helm chart" : `Upgrade ${releaseName || "release"}`}
        description={
          mode === "install"
            ? "Pick a chart, customize values, and helm install. Lumen shells out to your local helm CLI."
            : "Edit values or pin a new chart version, then helm upgrade. --atomic rolls back on failure."
        }
        icon={<Package className="size-4" />}
        actions={
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                navigate(`/cluster/${encodeURIComponent(context)}/helm`)
              }
            >
              <ArrowLeft className="size-3.5" /> back
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => startRun(true)}
              disabled={errors.length > 0}
              title="render manifest without applying"
            >
              dry run
            </Button>
            <Button
              size="sm"
              onClick={() => startRun(false)}
              disabled={errors.length > 0 || readOnly}
              title={readOnly ? capability.reason : undefined}
            >
              {readOnly && <Lock className="size-3.5" />}
              {mode === "install" ? "install" : "upgrade"}
            </Button>
          </div>
        }
      />

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="flex flex-col gap-4">
          <ChartPickerPanel
            mode={mode}
            spec={spec}
            setSpec={setSpec}
            disabled={mode === "upgrade" && !existing.data}
            onChartChosen={async (chart, version) => {
              if (valuesPrefilled) return;
              try {
                const yaml = await k8s.helmShowValues(chart, version || undefined);
                setSpec((s) => ({ ...s, valuesYaml: yaml }));
                setValuesPrefilled(true);
              } catch {
                // helm show values fails silently — the user can still type.
              }
            }}
          />

          <SectionPanel>
            <h2 className="mds-heading text-[14px] text-text-primary mb-3">Release</h2>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <Field label="release name">
                <Input
                  value={spec.release}
                  disabled={mode === "upgrade"}
                  onChange={(e) => setSpec((s) => ({ ...s, release: e.target.value }))}
                  placeholder="my-release"
                />
              </Field>
              <Field label="namespace">
                <select
                  value={spec.namespace}
                  disabled={mode === "upgrade"}
                  onChange={(e) =>
                    setSpec((s) => ({ ...s, namespace: e.target.value }))
                  }
                  className="h-9 w-full rounded-control border border-border-default bg-elevated px-3 text-xs text-text-primary disabled:opacity-60"
                >
                  {(namespaces.data ?? []).length === 0 && (
                    <option value={spec.namespace}>{spec.namespace}</option>
                  )}
                  {(namespaces.data ?? []).map((ns) => (
                    <option key={ns} value={ns}>
                      {ns}
                    </option>
                  ))}
                </select>
              </Field>
              {mode === "install" && (
                <Field label="namespace handling">
                  <label className="flex h-9 items-center gap-2 text-[12px] text-text-secondary">
                    <input
                      type="checkbox"
                      checked={spec.createNamespace}
                      onChange={(e) =>
                        setSpec((s) => ({ ...s, createNamespace: e.target.checked }))
                      }
                    />
                    create namespace if missing
                  </label>
                </Field>
              )}
              {mode === "upgrade" && (
                <Field label="atomic">
                  <label className="flex h-9 items-center gap-2 text-[12px] text-text-secondary">
                    <input
                      type="checkbox"
                      checked={spec.atomic}
                      onChange={(e) =>
                        setSpec((s) => ({ ...s, atomic: e.target.checked }))
                      }
                    />
                    rollback on failure (--atomic)
                  </label>
                </Field>
              )}
              <Field label="wait">
                <label className="flex h-9 items-center gap-2 text-[12px] text-text-secondary">
                  <input
                    type="checkbox"
                    checked={spec.wait}
                    onChange={(e) =>
                      setSpec((s) => ({ ...s, wait: e.target.checked }))
                    }
                  />
                  wait for resources to become ready
                </label>
              </Field>
            </div>
          </SectionPanel>

          <SectionPanel>
            <div className="mb-2 flex items-center justify-between">
              <h2 className="mds-heading text-[14px] text-text-primary">Values</h2>
              <span className="text-[11px] text-text-muted">
                YAML — passed via <code className="font-mono">--values -</code>
              </span>
            </div>
            <textarea
              value={spec.valuesYaml}
              onChange={(e) => {
                setSpec((s) => ({ ...s, valuesYaml: e.target.value }));
                setValuesPrefilled(true);
              }}
              spellCheck={false}
              className="h-[28rem] w-full rounded border border-border-subtle bg-code-surface p-3 font-mono text-[12px] leading-relaxed text-text-primary outline-none focus:ring-1 focus:ring-accent-primary/40"
              placeholder={
                mode === "install"
                  ? "# pick a chart above to load its default values, or paste your own here"
                  : "# current release values — edit then helm upgrade"
              }
            />
          </SectionPanel>

          {errors.length > 0 && (
            <div className="rounded-panel border border-warning/40 bg-warning-soft p-3 text-[11px] text-warning">
              <ul className="list-disc pl-4">
                {errors.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            </div>
          )}

          {readOnly && (
            <div className="rounded-panel border border-warning/40 bg-warning-soft p-3 text-[11px] text-warning">
              {capability.reason}. You can still prepare and dry-run this release.
            </div>
          )}
        </div>

        <div>
          <SectionPanel className="sticky top-4">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="mds-heading text-[14px] text-text-primary">Review</h2>
              <span className="font-mono text-[10px] text-text-muted">
                shell preview
              </span>
            </div>
            <pre className="max-h-[25vh] overflow-auto rounded border border-border-subtle bg-code-surface p-3 font-mono text-[11px] leading-relaxed text-text-primary whitespace-pre-wrap break-all">
              {previewCmd}
            </pre>
            <p className="mt-3 text-[11px] text-text-muted">
              Lumen shells out to <code className="font-mono">helm</code>; values are piped to stdin.
              Use the dry-run button to render the manifest before applying.
            </p>
            {dryRunPending && (
              <div className="mt-2 rounded border border-accent-primary/30 bg-accent-primary-soft p-2 text-[11px] text-accent-primary">
                <CheckCircle2 className="mr-1 inline size-3" />
                dry run started — see streaming output in the dialog.
              </div>
            )}
          </SectionPanel>
        </div>
      </div>

      {pendingAction && (
        <HelmActionDialog
          action={pendingAction}
          context={context}
          onClose={() => setPendingAction(null)}
          onSuccess={() => {
            navigate(
              `/cluster/${encodeURIComponent(context)}/helm?selected=${encodeURIComponent(spec.namespace)}/${encodeURIComponent(spec.release)}`,
            );
          }}
        />
      )}
      {pendingPreviewAction &&
        (pendingPreviewAction.kind === "install" || pendingPreviewAction.kind === "upgrade") && (
          <PreflightPreviewDialog
            context={context}
            open
            title={`preflight helm ${pendingPreviewAction.kind}`}
            description="This preview is local and conservative. Use dry run to inspect the rendered manifest before executing."
            impact={helmPreflight}
            confirmText={`${pendingPreviewAction.request.namespace || "default"}/${pendingPreviewAction.request.release}`}
            confirmLabel={pendingPreviewAction.kind}
            busy={readOnly}
            onCancel={() => setPendingPreviewAction(null)}
            onConfirm={() => {
              setPendingAction(pendingPreviewAction);
              setPendingPreviewAction(null);
            }}
          />
        )}
      {dryRunPending && (
        <HelmActionDialog
          action={dryRunPending}
          context={context}
          onClose={() => setDryRunPending(null)}
        />
      )}
    </LumenPage>
  );
}

function ChartPickerPanel({
  mode,
  spec,
  setSpec,
  disabled,
  onChartChosen,
}: {
  mode: WizardMode;
  spec: HelmWizardSpec;
  setSpec: React.Dispatch<React.SetStateAction<HelmWizardSpec>>;
  disabled: boolean;
  onChartChosen: (chart: string, version: string) => void;
}) {
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setDebounced(query), 300);
    return () => clearTimeout(t);
  }, [query]);

  const search = useQuery({
    queryKey: ["helm-search", debounced],
    queryFn: () => k8s.helmSearchRepo(debounced),
    staleTime: 60_000,
  });

  const grouped = useMemo(
    () => groupSearchHits(search.data ?? []),
    [search.data],
  );

  const chartGroup = useMemo(
    () => grouped.find((g) => g.name === spec.chart) ?? null,
    [grouped, spec.chart],
  );

  return (
    <SectionPanel>
      <div className="mb-3 flex items-center justify-between">
        <h2 className="mds-heading text-[14px] text-text-primary">Chart</h2>
        <span className="text-[11px] text-text-muted">
          {mode === "upgrade" ? "swap to a new chart or version" : "search configured helm repos"}
        </span>
      </div>

      <div className="flex flex-col gap-3">
        <HelmRepositories />
        {search.error && <p role="alert" className="text-[11px] text-error">Chart search failed. Check your local Helm configuration or enter a chart reference below.</p>}
        <div className="flex items-center gap-2 rounded border border-border-default bg-elevated px-2 h-9">
          <Search className="size-3.5 text-text-muted" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="search charts… (e.g. redis, ingress-nginx)"
            disabled={disabled}
            className="flex-1 bg-transparent text-[12px] text-text-primary placeholder:text-text-muted outline-none"
          />
          {search.isFetching && (
            <Sparkles className="size-3 animate-pulse text-accent-primary" />
          )}
        </div>

        {grouped.length > 0 && (
          <div className="max-h-48 overflow-auto rounded border border-border-subtle">
            {grouped.slice(0, 50).map((g) => (
              <button
                key={g.name}
                type="button"
                onClick={() => {
                  setSpec((s) => ({ ...s, chart: g.name, version: g.versions[0] ?? "" }));
                  onChartChosen(g.name, g.versions[0] ?? "");
                }}
                className={cn(
                  "flex w-full flex-col items-start gap-0.5 border-b border-border-subtle px-3 py-2 text-left text-[12px] last:border-b-0 hover:bg-elevated",
                  spec.chart === g.name && "bg-accent-primary-soft",
                )}
              >
                <div className="flex w-full items-center gap-2">
                  <span className="font-mono text-text-primary">{g.name}</span>
                  <span className="ml-auto text-[11px] text-text-muted">
                    {g.versions.length} version{g.versions.length === 1 ? "" : "s"}
                  </span>
                </div>
                {g.description && (
                  <span className="line-clamp-1 text-[11px] text-text-muted">{g.description}</span>
                )}
              </button>
            ))}
          </div>
        )}

        {!search.isFetching && (search.data ?? []).length === 0 && debounced.length > 0 && (
          <p className="text-[11px] text-text-muted">
            no matches — fall back to manual entry below.
          </p>
        )}

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <Field label="chart (manual)">
            <Input
              value={spec.chart}
              onChange={(e) => setSpec((s) => ({ ...s, chart: e.target.value }))}
              placeholder="bitnami/redis or oci://registry/foo"
              disabled={disabled}
            />
          </Field>
          <Field label="version">
            {chartGroup ? (
              <select
                value={spec.version}
                onChange={(e) => {
                  setSpec((s) => ({ ...s, version: e.target.value }));
                  onChartChosen(spec.chart, e.target.value);
                }}
                disabled={disabled}
                className="h-9 w-full rounded-control border border-border-default bg-elevated px-3 text-xs text-text-primary disabled:opacity-60"
              >
                <option value="">latest</option>
                {chartGroup.versions.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
            ) : (
              <Input
                value={spec.version}
                onChange={(e) => setSpec((s) => ({ ...s, version: e.target.value }))}
                placeholder="leave blank for latest"
                disabled={disabled}
              />
            )}
          </Field>
        </div>
      </div>
    </SectionPanel>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-[11px] uppercase tracking-wide text-text-muted">
        {label}
      </span>
      {children}
    </label>
  );
}
