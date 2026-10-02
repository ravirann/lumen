import { useEffect } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { fetchGpuScheduling } from "@/lib/gpuScheduling";
import type { Explanation, ResourceRef } from "@/lib/gpuTypes";
import { redactIncidentReportText } from "@/lib/incidentReport";
import { SectionPanel } from "@/components/lumen/page";
import { Button } from "@/components/ui/button";

type Props = { context: string; namespace: string; pod: string; expectedUid: string; onEvidence: (items: Explanation[]) => void; onSources?: (lines: string[]) => void };
const EMPTY: Explanation[] = [];
const NO_SOURCES: string[] = [];
function resourceHref(context: string, ref: ResourceRef): string | null {
  const base = `/cluster/${encodeURIComponent(context)}`;
  const kind = ref.kind.toLowerCase();
  const params = new URLSearchParams({ ...(ref.namespace ? { ns: ref.namespace } : {}), q: ref.name });
  if (kind === "node") return `${base}/nodes?${params}`;
  if (kind === "resourceclaim") return `${base}/device-resources?${params}`;
  const slugs: Record<string, string> = { pod: "pods", job: "jobs", cronjob: "cronjobs", deployment: "deployments", replicaset: "replicasets", statefulset: "statefulsets", daemonset: "daemonsets", persistentvolumeclaim: "pvcs" };
  return slugs[kind] ? `${base}/workloads/${slugs[kind]}?${params}` : null;
}
export function GpuSchedulingPanel({ context, namespace, pod, expectedUid, onEvidence, onSources }: Props) {
  const enabled = !!context && !!namespace && !!pod && !!expectedUid;
  const query = useQuery({ queryKey: ["gpu-scheduling", context, namespace, pod, expectedUid], queryFn: () => fetchGpuScheduling(context, namespace, pod, expectedUid), enabled, retry: false, refetchOnWindowFocus: false });
  const matches = query.data?.pod.uid === expectedUid && query.data.pod.name === pod && query.data.pod.namespace === namespace;
  const evidence = enabled && query.isSuccess && !query.isFetching && matches ? query.data.explanations : EMPTY;
  useEffect(() => { onEvidence(evidence); }, [evidence, onEvidence]);
  useEffect(() => {
    onSources?.(enabled && query.isSuccess && !query.isFetching && matches
      ? Object.entries(query.data.sources).map(([name, source]) => `${name}: ${source.state} · ${source.complete ? "complete" : "incomplete"} · captured ${source.captured_at}${source.message ? ` · ${redactIncidentReportText(source.message)}` : ""}`)
      : NO_SOURCES);
  }, [enabled, matches, query.data, query.isFetching, query.isSuccess, onSources]);
  useEffect(() => () => { onEvidence(EMPTY); onSources?.(NO_SOURCES); }, [onEvidence, onSources]);
  return <SectionPanel><section aria-label="GPU scheduling evidence" className="space-y-3">
    <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="text-sm font-semibold text-text-primary">Explain scheduling</h3><Button variant="outline" disabled={!enabled || query.isFetching} onClick={() => void query.refetch()}>refresh scheduling</Button></div>
    {!enabled ? <p className="text-xs text-text-secondary">Scheduling evidence unavailable: namespace and current pod UID are required.</p>
      : query.isError ? <p role="status" className="text-xs text-text-secondary">Scheduling evidence unavailable: access denied, request failed, or selected pod replaced. Refresh the investigation to verify its identity.</p>
      : query.isPending || query.isFetching ? <p role="status" className="text-xs text-text-secondary">Checking current pod identity and scheduling evidence…</p>
      : !matches ? <p role="status" className="text-xs text-text-secondary">Pod identity changed; scheduling evidence excluded.</p>
      : <>
        <p className="text-xs text-text-secondary">Observed statements, inferred configuration, and unknown outcomes are separate. Collection is not an atomic snapshot; no placement simulation or queue ETA is provided.</p>
        <ul className="space-y-1 text-xs text-text-secondary">{Object.entries(query.data.sources).map(([name, source]) => <li key={name}>{name}: {source.state} · {source.complete ? "complete" : "incomplete"} · captured {source.captured_at}{source.message ? ` · ${redactIncidentReportText(source.message)}` : ""}</li>)}</ul>
        {(["admission", "scheduling", "startup"] as const).map((stage) => <div key={stage} className="space-y-1"><h4 className="text-sm font-medium text-text-primary">{stage[0].toUpperCase() + stage.slice(1)}</h4>
          {!evidence.some((item) => item.stage === stage) && <p className="text-xs text-text-secondary">unknown · No current {stage} blocker established by this capture.</p>}
          {evidence.filter((item) => item.stage === stage).map((item, index) => <div key={index} className="text-xs text-text-secondary"><p>{item.confidence} · captured {item.captured_at}{item.transition_time ? ` · transitioned ${item.transition_time}` : ""}{item.observed_generation != null ? ` · observed generation ${item.observed_generation}` : ""}</p><p className="break-words">{redactIncidentReportText(item.message)}</p><ul>{item.sources.map((ref) => <li key={`${ref.kind}/${ref.namespace}/${ref.name}/${ref.uid}`}>{resourceHref(context, ref) ? <Link className="underline break-all" to={resourceHref(context, ref)!}>{ref.kind}/{ref.name}</Link> : <span className="break-all">{ref.kind}/{ref.name}</span>} · UID {ref.uid}</li>)}</ul></div>)}
        </div>)}
      </>}
  </section></SectionPanel>;
}
