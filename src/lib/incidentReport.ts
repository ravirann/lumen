import type { EventLine, WorkloadKind } from "@/lib/k8s";
import type { RolloutTimelineEntry } from "@/lib/rolloutTimeline";
import type { TriageIssue, TriageSeverity } from "@/lib/triage";
import type { LogEvidence } from "@/lib/logEvidence";

const DEFAULT_NEXT_CHECKS = [
  "Correlate warnings with rollout and activity timeline timestamps",
  "Capture current resource YAML and relevant logs before remediation",
];

const SEVERITY_ORDER: Record<TriageSeverity, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  low: 3,
};

export type IncidentReportResource = {
  kind: WorkloadKind | string;
  namespace: string | null;
  name: string;
};

export type IncidentReportWarningEvent = EventLine & {
  count?: number | null;
};

export type IncidentReportInput = {
  clusterContext: string;
  namespace?: string | null;
  generatedAt?: Date;
  selectedResource?: IncidentReportResource | null;
  triageIssues?: TriageIssue[];
  warningEvents?: IncidentReportWarningEvent[];
  rolloutEntries?: RolloutTimelineEntry[];
  manualNotes?: string;
  investigation?: { startedAt: string; sources: string[]; observations: string[]; logEvidence?: LogEvidence };
};

export type IncidentReportData = {
  investigation?: { startedAt: string; sources: string[]; observations: string[]; logEvidence?: LogEvidence };
  generatedAtIso: string;
  scope: {
    clusterContext: string;
    namespace: string;
    selectedResource: string;
  };
  summary: Record<TriageSeverity | "totalIssues" | "warningEvents" | "rolloutNotes", number>;
  triageIssues: TriageIssue[];
  warningEvents: Array<{
    timestamp: string;
    reason: string;
    involved: string;
    message: string;
    count: number;
  }>;
  rolloutNotes: Array<{
    timestamp: string;
    severity: string;
    resource: string;
    title: string;
    description: string;
    details: string[];
  }>;
  nextChecks: string[];
  manualNotes: string;
};

function formatResource(resource: IncidentReportResource | null | undefined): string {
  if (!resource) return "none";
  const namespace = resource.namespace?.trim();
  return namespace
    ? `${resource.kind}/${namespace}/${resource.name}`
    : `${resource.kind}/${resource.name}`;
}

function safeIso(date: Date | undefined): string {
  const value = date ?? new Date();
  return Number.isNaN(value.getTime()) ? new Date().toISOString() : value.toISOString();
}

function warningTimestamp(event: EventLine): number {
  const parsed = Date.parse(event.ts ?? "");
  return Number.isFinite(parsed) ? parsed : 0;
}

function uniqueNonEmpty(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    out.push(trimmed);
  }
  return out;
}

function cleanLine(value: string | null | undefined): string {
  return redactIncidentReportText(value ?? "").replace(/\s+/g, " ").trim();
}
function finalLogExcerpt(value: string): { text: string; truncated: boolean } {
  const redacted = redactIncidentReportText(value);
  const text = redacted.split(/\r?\n/).slice(0, 200).join("\n").slice(0, 20_000);
  return { text, truncated: text !== redacted };
}

function normalizeLogEvidence(log: LogEvidence): LogEvidence {
  const bounded = finalLogExcerpt(log.text);
  return {
    ...log,
    context: cleanLine(log.context),
    namespace: cleanLine(log.namespace),
    pod: cleanLine(log.pod),
    podUid: log.podUid ? cleanLine(log.podUid) : null,
    container: cleanLine(log.container),
    text: bounded.text,
    truncated: log.truncated || bounded.truncated,
    note: cleanLine(log.note),
  };
}

export function buildIncidentReportData(input: IncidentReportInput): IncidentReportData {
  const generatedAtIso = safeIso(input.generatedAt);
  const namespace = input.namespace?.trim() || "all namespaces";
  const triageIssues = [...(input.triageIssues ?? [])].sort(
    (a, b) =>
      SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity] ||
      a.title.localeCompare(b.title) ||
      a.resource.name.localeCompare(b.resource.name),
  );
  const warningEvents = (input.warningEvents ?? [])
    .filter((event) => (event.type_ ?? "").toLowerCase() === "warning")
    .sort((a, b) => warningTimestamp(b) - warningTimestamp(a))
    .slice(0, 12)
    .map((event) => ({
      timestamp: event.ts ?? "",
      reason: cleanLine(event.reason),
      involved: cleanLine(event.involved),
      message: cleanLine(event.message),
      count: event.count ?? 1,
    }));
  const rolloutNotes = (input.rolloutEntries ?? []).slice(0, 12).map((entry) => ({
    timestamp: new Date(entry.timestampMs).toISOString(),
    severity: entry.severity,
    resource: formatResource({
      kind: entry.resourceKind,
      namespace: entry.namespace || null,
      name: entry.resourceName,
    }),
    title: cleanLine(entry.title),
    description: cleanLine(entry.description),
    details: entry.details.map(cleanLine).filter(Boolean),
  }));
  const summary = triageIssues.reduce<IncidentReportData["summary"]>(
    (acc, issue) => {
      acc.totalIssues += 1;
      acc[issue.severity] += 1;
      return acc;
    },
    {
      totalIssues: 0,
      critical: 0,
      high: 0,
      medium: 0,
      low: 0,
      warningEvents: warningEvents.length,
      rolloutNotes: rolloutNotes.length,
    },
  );

  return {
    investigation: input.investigation ? {
      startedAt: cleanLine(input.investigation.startedAt),
      sources: input.investigation.sources.map(cleanLine),
      observations: input.investigation.observations.map(cleanLine),
      logEvidence: input.investigation.logEvidence
        ? normalizeLogEvidence(input.investigation.logEvidence)
        : undefined,
    } : undefined,
    generatedAtIso,
    scope: {
      clusterContext: cleanLine(input.clusterContext || "current-context"),
      namespace,
      selectedResource: formatResource(input.selectedResource),
    },
    summary,
    triageIssues,
    warningEvents,
    rolloutNotes,
    nextChecks: uniqueNonEmpty([
      ...triageIssues.flatMap((issue) => issue.nextActions),
      ...DEFAULT_NEXT_CHECKS,
    ]),
    manualNotes: redactIncidentReportText(input.manualNotes ?? "").trim(),
  };
}

function bullet(line: string): string {
  return `- ${line}`;
}

function renderList(title: string, lines: string[]): string[] {
  return [
    title,
    "",
    ...(lines.length ? lines.map(bullet) : ["- none captured"]),
    "",
  ];
}

export function renderIncidentReportMarkdown(report: IncidentReportData): string {
  const lines: string[] = [
    `# Incident Report - ${report.scope.clusterContext}`,
    "",
    "## Context",
    "",
    `- **Cluster:** ${report.scope.clusterContext}`,
    `- **Namespace:** ${report.scope.namespace}`,
    `- **Timestamp:** ${report.generatedAtIso}`,
    `- **Selected resource:** ${report.scope.selectedResource}`,
    "",
    "## Triage Summary",
    "",
    `- **Total issues:** ${report.summary.totalIssues}`,
    `- **Critical:** ${report.summary.critical}`,
    `- **High:** ${report.summary.high}`,
    `- **Medium:** ${report.summary.medium}`,
    `- **Low:** ${report.summary.low}`,
    `- **Warning events:** ${report.summary.warningEvents}`,
    `- **Rollout notes:** ${report.summary.rolloutNotes}`,
    "",
  ];

  let logTextIndex: number | undefined;
  if (report.investigation) {
    lines.push("## Investigation capture", "", `Started: ${report.investigation.startedAt}`, "", ...renderList("Sources and freshness:", report.investigation.sources), ...renderList("Observed evidence and limitations:", report.investigation.observations));
    const log = report.investigation.logEvidence;
    if (log) {
      lines.push("### Selected log evidence", "",
        `- **Status:** ${log.status}${log.included ? " · included" : " · excluded"}`,
        `- **Identity:** ${log.context} · ${log.namespace}/${log.pod} · UID ${log.podUid ?? "unavailable"} · ${log.container} · ${log.instance}`,
        `- **Captured:** ${log.capturedAt}; bounds ${log.lineLimit} lines / ${log.charLimit} characters; truncated ${log.truncated ? "yes" : "no"}`,
        `- **Note:** ${log.note}`, "");
      if (log.included) {
        lines.push("```text");
        logTextIndex = lines.length;
        lines.push(log.text, "```", "");
      }
    }
  }

  if (report.triageIssues.length === 0) {
    lines.push("No active triage issues were captured.", "");
  } else {
    for (const issue of report.triageIssues) {
      lines.push(
        `### ${issue.severity} - ${redactIncidentReportText(issue.title)}`,
        "",
        `- **Group:** ${issue.group}`,
        `- **Resource:** ${formatResource(issue.resource)}`,
        "",
        ...renderList("Evidence:", issue.evidence.map(redactIncidentReportText)),
        ...renderList("Next checks:", issue.nextActions.map(redactIncidentReportText)),
      );
    }
  }

  lines.push("## Warning Events", "");
  if (report.warningEvents.length === 0) {
    lines.push("- none captured", "");
  } else {
    for (const event of report.warningEvents) {
      lines.push(
        `- **${event.reason || "Warning"}** ${event.involved || "unknown resource"} (${event.timestamp || "unknown time"}, count ${event.count}): ${event.message}`,
      );
    }
    lines.push("");
  }

  lines.push("## Rollout / Timeline Notes", "");
  if (report.rolloutNotes.length === 0) {
    lines.push("- no rollout notes available", "");
  } else {
    for (const entry of report.rolloutNotes) {
      const details = entry.details.length ? ` Details: ${entry.details.join("; ")}.` : "";
      lines.push(
        `- **${entry.severity}** ${entry.title} - ${entry.resource} at ${entry.timestamp}: ${entry.description}.${details}`,
      );
    }
    lines.push("");
  }

  lines.push(...renderList("## Next Checks", report.nextChecks.map(redactIncidentReportText)));
  lines.push("## Manual Notes", "", report.manualNotes || "_none_", "");

  if (logTextIndex !== undefined) {
    // Redact and bound the excerpt last. Redacting the complete report afterward
    // could expand a marker clipped at the character limit beyond its bound.
    const before = redactIncidentReportText(lines.slice(0, logTextIndex).join("\n"));
    const excerpt = finalLogExcerpt(lines[logTextIndex]).text;
    const after = redactIncidentReportText(lines.slice(logTextIndex + 1).join("\n"));
    return [before, excerpt, after].join("\n").trimEnd() + "\n";
  }
  return redactIncidentReportText(lines.join("\n")).trimEnd() + "\n";
}

export function redactIncidentReportText(text: string): string {
  let out = text;
  // Controller and scheduler prose can contain inline or quoted credentials.
  out = out.replace(/\b((?:token|password|client[_-]?secret|api[_-]?key|access[_-]?token|refresh[_-]?token)\s*[:=]\s*)(?:"[^"\n]*"|'[^'\n]*'|[^\s,;]+)/gi, "$1[REDACTED]");
  out = out.replace(/(authorization:\s*bearer\s+)[^\s]+/gi, "$1[REDACTED]");
  out = out.replace(/\b(bearer\s+)[A-Za-z0-9._~+/=-]+/gi, "$1[REDACTED]");
  out = out.replace(
    /^(\s*(?:token|client-key-data|client-certificate-data|password|client-secret|client_secret|api-?key|api_key|access-token|refresh-token)\s*:\s*).+$/gim,
    "$1[REDACTED]",
  );
  out = out.replace(
    /^(\s*[A-Z0-9_]*(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD|CLIENT[_-]?SECRET)[A-Z0-9_]*\s*=\s*).+$/gim,
    "$1[REDACTED]",
  );

  if (/^\s*kind:\s*Secret\s*$/im.test(out)) {
    const lines = out.split("\n");
    const redacted: string[] = [];
    let redactingSecretMap = false;
    let redactingIndent = 0;
    for (const line of lines) {
      const mapMatch = line.match(/^(\s*)(data|stringData):\s*$/);
      if (mapMatch) {
        redacted.push(`${mapMatch[1]}${mapMatch[2]}: [REDACTED]`);
        redactingSecretMap = true;
        redactingIndent = mapMatch[1].length;
        continue;
      }
      if (redactingSecretMap) {
        const indent = line.match(/^(\s*)/)?.[1].length ?? 0;
        if (line.trim() && indent > redactingIndent) continue;
        redactingSecretMap = false;
      }
      redacted.push(line);
    }
    out = redacted.join("\n");
  }

  return out;
}

function filenamePart(value: string | null | undefined): string {
  return (value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function timestampPart(date: Date | undefined): string {
  const iso = safeIso(date);
  return iso
    .replace(/[-:]/g, "")
    .replace(/\.\d{3}Z$/, "")
    .replace("T", "-");
}

export function incidentReportFilename(input: IncidentReportInput): string {
  const parts = [
    "incident",
    filenamePart(input.clusterContext),
    filenamePart(input.namespace),
    input.selectedResource ? filenamePart(input.selectedResource.kind) : "",
    input.selectedResource ? filenamePart(input.selectedResource.name) : "",
    timestampPart(input.generatedAt),
  ].filter(Boolean);
  return `${parts.join("-")}.md`;
}
