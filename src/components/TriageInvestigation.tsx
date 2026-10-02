import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { useQuery, type UseQueryResult } from "@tanstack/react-query";
import { k8s, type EventSummary, type ResourceDetail, type WorkloadKind } from "@/lib/k8s";
import type { TriageIssue } from "@/lib/triage";
import { redactIncidentReportText } from "@/lib/incidentReport";
import { Button } from "@/components/ui/button";
import { SectionPanel } from "@/components/lumen/page";
import { IncidentReportDialog } from "@/components/IncidentReportDialog";
import { boundLogEvidenceText, captureIncidentLogEvidence, resourceUid, type LogEvidence } from "@/lib/logEvidence";

import { GpuSchedulingPanel } from "@/components/gpu/GpuSchedulingPanel";
import type { Explanation } from "@/lib/gpuTypes";

type Props = { context: string; issue: TriageIssue; startedAt: string; onClose: () => void };
const OWNER_KINDS = new Set(["deployment", "replicaset", "statefulset", "daemonset", "job", "cronjob", "replicationcontroller"]);
function ownerOf(resource?: ResourceDetail) {
  const ref = resource?.owner_refs.find((owner) => OWNER_KINDS.has(owner.kind.toLowerCase()));
  return ref ? { kind: ref.kind.toLowerCase() as WorkloadKind, name: ref.name } : null;
}
function matchingEvents(events: EventSummary[] | undefined, kind: string, name: string, uid: string | null) {
  return (events ?? []).filter((event) => event.involved_kind.toLowerCase() === kind.toLowerCase() && event.involved_name === name && !!uid && event.involved_uid === uid);
}
function source(label: string, query: Pick<UseQueryResult, "isPending" | "isError" | "dataUpdatedAt">, enabled = true) {
  if (!enabled) return `${label}: unavailable for this resource`;
  if (query.isError) return `${label}: unavailable (access denied or request failed)${query.dataUpdatedAt ? `; stale evidence retained from ${new Date(query.dataUpdatedAt).toISOString()}` : ""}`;
  if (query.isPending) return `${label}: loading`;
  return `${label}: captured ${new Date(query.dataUpdatedAt).toISOString()}`;
}

// Key the session so notes, container choices and pending results never cross targets.
export function TriageInvestigation(props: Props) {
  return <InvestigationSession key={JSON.stringify([props.context, props.issue.resource, props.startedAt])} {...props} />;
}
function InvestigationSession({ context, issue, startedAt, onClose }: Props) {
  const { kind, name } = issue.resource;
  const namespace = issue.resource.namespace ?? "";
  const [reportOpen, setReportOpen] = useState(false);
  const [chosenContainer, setChosenContainer] = useState<string | null>(null);
  const [previous, setPrevious] = useState(false);
  const [logEvidence, setLogEvidence] = useState<LogEvidence | undefined>();
  const [capturing, setCapturing] = useState(false);
  const captureVersion = useRef(0);
  const identity = [context, namespace, kind, name, startedAt];
  const resource = useQuery({ queryKey: ["investigation-resource", ...identity], queryFn: () => k8s.getResource(namespace, kind, name, context), retry: false });
  const pod = useQuery({ queryKey: ["investigation-pod", ...identity], queryFn: () => k8s.getPodDetails(context, namespace, name), enabled: kind === "pod" && !!namespace, retry: false });
  const events = useQuery({ queryKey: ["investigation-events", ...identity], queryFn: () => k8s.listEventsFor(namespace, kind, name, context), retry: false });
  const owner = ownerOf(resource.data);
  const ownerQuery = useQuery({ queryKey: ["investigation-owner", ...identity, owner], queryFn: () => k8s.getResource(namespace, owner!.kind, owner!.name, context), enabled: !!owner, retry: false });
  const controller = ownerOf(ownerQuery.data);
  const controllerQuery = useQuery({ queryKey: ["investigation-controller", ...identity, controller], queryFn: () => k8s.getResource(namespace, controller!.kind, controller!.name, context), enabled: !!controller, retry: false });
  const rolloutTarget = controller ?? owner ?? (OWNER_KINDS.has(kind) ? { kind, name } : null);
  const ownerEvents = useQuery({ queryKey: ["investigation-owner-events", ...identity, rolloutTarget], queryFn: () => k8s.listEventsFor(namespace, rolloutTarget!.kind, rolloutTarget!.name, context), enabled: !!rolloutTarget, retry: false });
  const containers = pod.data?.containers ?? [];
  const suggested = [...containers].sort((a, b) => Number(a.ready) - Number(b.ready) || b.restart_count - a.restart_count)[0];
  const container = containers.find((entry) => entry.name === chosenContainer) ?? suggested;
  const uid = resourceUid(resource.data);
  const schedulingIdentity = JSON.stringify([context, namespace, name, uid]);
  const [scheduling, setScheduling] = useState<{ identity: string; items: Explanation[] }>();
  const onSchedulingEvidence = useCallback((items: Explanation[]) => setScheduling({ identity: schedulingIdentity, items }), [schedulingIdentity]);
  const [schedulingSources, setSchedulingSources] = useState<{ identity: string; lines: string[] }>();
  const onSchedulingSources = useCallback((lines: string[]) => setSchedulingSources({ identity: schedulingIdentity, lines }), [schedulingIdentity]);
  const schedulingEnabled = kind === "pod" && !!namespace && !!uid && resource.isSuccess && !resource.isFetching;
  const schedulingObservations = schedulingEnabled && scheduling?.identity === schedulingIdentity ? scheduling.items.map((item) => `${item.stage} · ${item.confidence} · captured ${item.captured_at}${item.transition_time ? ` · transitioned ${item.transition_time}` : ""}${item.observed_generation != null ? ` · observed generation ${item.observed_generation}` : ""} · ${redactIncidentReportText(item.message)}${item.sources.length ? ` · sources: ${item.sources.map((ref) => `${ref.kind}/${ref.namespace ?? "cluster"}/${ref.name} UID ${ref.uid}`).join(", ")}` : ""}`) : [];
  // A query refresh can change identity or the suggested container without a
  // selection event. Invalidate that capture just as an explicit selection does.
  useEffect(() => {
    captureVersion.current += 1;
    setCapturing(false);
    setLogEvidence(undefined);
    return () => { captureVersion.current += 1; };
  }, [uid, container?.name, previous]);
  const relatedEvents = matchingEvents(events.data, kind, name, uid);
  const rolloutUid = resourceUid(controllerQuery.data ?? ownerQuery.data ?? (OWNER_KINDS.has(kind) ? resource.data : undefined));
  const rolloutEvents = rolloutTarget ? matchingEvents(ownerEvents.data, rolloutTarget.kind, rolloutTarget.name, rolloutUid) : [];
  const owners = [OWNER_KINDS.has(kind) ? resource.data : undefined, ownerQuery.data, controllerQuery.data].filter((entry): entry is ResourceDetail => !!entry);
  const observations = owners.map((entry) => `${entry.summary.kind}/${entry.summary.name} · ${entry.summary.ready} ready · ${entry.summary.health} (current snapshot)`);
  if (!owners.length) observations.push("Owner rollout evidence unavailable; no controller snapshot captured.");
  observations.push("Historical rollout revisions unavailable in this capture; current snapshots do not establish a rollout cause.");
  observations.push(uid ? `Events without matching involved-object UID ${uid} are excluded.` : "Resource UID unavailable; name-only events are unverified and excluded.");
  observations.push("Log evidence is included only after explicit bounded capture and may be unavailable after retention or eviction; redaction is best effort.");
  if (kind === "pod" && pod.isSuccess && !container) observations.push("Container log selection unavailable: no containers returned.");
  if (container) observations.push(`Selected container: ${container.name}; ${container.state}; ${container.restart_count} restarts.`);
  for (const event of rolloutEvents) observations.push(`Owner event ${event.involved_kind}/${event.involved_name}: ${event.ts ?? "unknown time"} · ${event.reason} · ${redactIncidentReportText(event.message)}`);
  const sources = [...(schedulingEnabled && schedulingSources?.identity === schedulingIdentity ? schedulingSources.lines : []), source("Resource", resource), source("Related events", events), source("Pod containers", pod, kind === "pod" && !!namespace), source("Owner snapshot", ownerQuery, !!owner), source("Controller snapshot", controllerQuery, !!controller), source("Owner events", ownerEvents, !!rolloutTarget)];
  const loading = resource.isPending || events.isPending || (kind === "pod" && !!namespace && pod.isPending) || (!!owner && ownerQuery.isPending) || (!!controller && controllerQuery.isPending) || (!!rolloutTarget && ownerEvents.isPending);
  function logs(previous: boolean) {
    const params = new URLSearchParams({ ns: namespace, kind: "pod", name, c: container!.name, startedAt });
    if (previous) params.set("previous", "true");
    return `/cluster/${encodeURIComponent(context)}/logs?${params}`;
  }
  async function captureLogs() {
    if (!container) return;
    const capturedIdentity = { context, namespace, pod: name, podUid: uid, container: container.name, previous };
    const version = ++captureVersion.current;
    setCapturing(true);
    const evidence = await captureIncidentLogEvidence(capturedIdentity);
    if (captureVersion.current !== version) return;
    setLogEvidence(evidence);
    setCapturing(false);
  }
  return <SectionPanel>
    <section aria-label="Selected investigation" className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div><h2 className="text-base font-semibold text-text-primary">Investigate {kind}/{name}</h2><p className="text-xs text-text-secondary">{context} · {namespace || "cluster scoped"} · {kind}/{name} · started {startedAt}</p></div>
        <div className="flex gap-2"><Button variant="outline" onClick={() => setReportOpen(true)} disabled={loading}>export investigation</Button><Button variant="outline" onClick={onClose}>close investigation</Button></div>
      </div>
      <ul className="text-xs text-text-secondary space-y-1">{sources.map((line) => <li key={line}>{line}</li>)}</ul>
      {kind === "pod" && container && <div className="flex flex-wrap items-center gap-2">
        <label className="text-xs text-text-secondary">Container <select aria-label="Investigation container" value={container.name} onChange={(event) => { captureVersion.current += 1; setCapturing(false); setChosenContainer(event.target.value); setLogEvidence(undefined); }} className="rounded-control border border-border-default bg-elevated p-1">{containers.map((entry) => <option key={entry.name} value={entry.name}>{entry.name} · {entry.state} · {entry.restart_count} restarts</option>)}</select></label>
        <Button asChild variant="outline"><Link to={logs(false)}>current logs · {container.name}</Link></Button>
        {container.restart_count > 0 && <Button asChild variant="outline"><Link to={logs(true)}>previous logs · {container.name}</Link></Button>}
        <label className="text-xs"><input type="checkbox" checked={previous} onChange={(event) => { captureVersion.current += 1; setCapturing(false); setPrevious(event.target.checked); setLogEvidence(undefined); }} /> capture previous instance</label>
        <Button variant="outline" disabled={capturing} onClick={() => void captureLogs()}>{capturing ? "capturing…" : "capture bounded logs"}</Button>
      </div>}
      {logEvidence && <div className="space-y-2 rounded-control border border-border-default p-3"><p className="text-xs text-text-secondary">{logEvidence.note} · max {logEvidence.lineLimit} lines / {logEvidence.charLimit} characters{logEvidence.truncated ? " · truncated" : ""}</p>{logEvidence.status === "captured" && <><textarea aria-label="Log evidence preview" value={logEvidence.text} onChange={(event) => { const text = boundLogEvidenceText(event.target.value); setLogEvidence({ ...logEvidence, text, truncated: logEvidence.truncated || text !== event.target.value }); }} className="h-40 w-full rounded-control border bg-code-surface p-2 font-mono text-xs" /><label className="text-xs"><input type="checkbox" checked={logEvidence.included} onChange={(event) => setLogEvidence({ ...logEvidence, included: event.target.checked })} /> include edited excerpt in export</label></>}</div>}
      {schedulingEnabled && <GpuSchedulingPanel key={schedulingIdentity} context={context} namespace={namespace} pod={name} expectedUid={uid!} onEvidence={onSchedulingEvidence} onSources={onSchedulingSources} />}
      <div><h3 className="text-sm font-medium text-text-primary">Related events</h3><p className="text-xs text-text-secondary">Fetched for {namespace || "cluster"}/{kind}/{name}; event timestamps are shown below.</p>
        {events.isSuccess && relatedEvents.length === 0 && <p className="text-xs text-text-secondary">No related events returned.</p>}
        <ul className="space-y-1 text-xs text-text-secondary">{relatedEvents.map((event, index) => <li key={index}>{event.ts ?? "unknown time"} · {event.type_} · {event.reason}: {redactIncidentReportText(event.message)}</li>)}</ul>
      </div>
      <div><h3 className="text-sm font-medium text-text-primary">Owner and rollout evidence</h3><ul className="space-y-1 text-xs text-text-secondary">{observations.map((line, index) => <li key={index}>{redactIncidentReportText(line)}</li>)}</ul></div>
    </section>
    <IncidentReportDialog open={reportOpen} onClose={() => setReportOpen(false)} loading={loading} input={{ clusterContext: context, namespace, selectedResource: issue.resource, triageIssues: [issue], warningEvents: relatedEvents.map((event) => ({ ...event, kind: "Event", involved: `${event.involved_kind}/${event.involved_name}` })), rolloutEntries: [], investigation: { startedAt, sources, observations: [...observations, ...schedulingObservations, ...relatedEvents.map((event) => `Related event ${event.ts ?? "unknown time"} · ${event.type_} · ${event.reason}: ${redactIncidentReportText(event.message)}`)], logEvidence } }} />
  </SectionPanel>;
}
