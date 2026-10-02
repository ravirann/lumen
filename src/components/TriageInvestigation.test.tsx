import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { k8s, type PodDetails, type ResourceDetail } from "@/lib/k8s";
import type { TriageIssue } from "@/lib/triage";
import { buildIncidentReportData, renderIncidentReportMarkdown, type IncidentReportInput } from "@/lib/incidentReport";
import { fetchGpuScheduling } from "@/lib/gpuScheduling";
import { TriageInvestigation } from "./TriageInvestigation";
vi.mock("@/lib/gpuScheduling", () => ({ fetchGpuScheduling: vi.fn() }));
vi.mock("@/lib/k8s", () => ({ k8s: { getResource: vi.fn(), getPodDetails: vi.fn(), listEventsFor: vi.fn(), captureIncidentLogs: vi.fn() } }));
vi.mock("@/components/IncidentReportDialog", () => ({ IncidentReportDialog: ({ open, input }: { open: boolean; input: IncidentReportInput }) => open ? <pre data-testid="report">{renderIncidentReportMarkdown(buildIncidentReportData(input))}</pre> : null }));
const issue: TriageIssue = { id: "crash", group: "crashloop-restarts", severity: "high", title: "Pod restarting repeatedly", resource: { kind: "pod", namespace: "payments", name: "api" }, evidence: ["5 restarts"], nextActions: ["Check previous logs"] };
const detail = (kind: string, name: string, owners: { kind: string; name: string }[] = []): ResourceDetail => ({ summary: { kind, name, namespace: "payments", ready: "0/1", health: "degraded", labels: {}, age_seconds: 60 }, yaml: `metadata:\n  name: ${name}\n  uid: uid-${kind}-${name}`, owner_refs: owners } as ResourceDetail);
function renderInvestigation() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const Wrapper = ({ children }: { children: React.ReactNode }) => <QueryClientProvider client={client}><MemoryRouter>{children}</MemoryRouter></QueryClientProvider>;
  return { client, ...render(<TriageInvestigation context="prod" issue={issue} startedAt="2026-09-06T00:00:00Z" onClose={() => {}} />, { wrapper: Wrapper }) };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(fetchGpuScheduling).mockImplementation(async (_ctx, ns, name, uid) => ({ pod: { api_version: "v1", kind: "Pod", namespace: ns, name, uid }, sources: { nodes: { state: "forbidden", complete: false, captured_at: "2026-10-02T00:00:00Z", items: [], message: "access denied" } }, explanations: [{ stage: "scheduling", confidence: "observed", message: "CURRENT_GPU_EVIDENCE token=synthetic-gpu-secret", sources: [], captured_at: "2026-10-02T00:00:00Z", transition_time: null, observed_generation: null }] }));
  vi.mocked(k8s.getResource).mockImplementation(async (_ns, kind, name) => detail(kind, name, kind === "pod" ? [{ kind: "ReplicaSet", name: "api-rs" }] : kind === "replicaset" ? [{ kind: "Deployment", name: "api" }] : []));
  vi.mocked(k8s.getPodDetails).mockResolvedValue({ name: "api", namespace: "payments", containers: [{ name: "sidecar", ready: true, restart_count: 0, state: "running" }, { name: "worker", ready: false, restart_count: 5, state: "CrashLoopBackOff" }] } as PodDetails);
  vi.mocked(k8s.listEventsFor).mockImplementation(async (_ns, kind, name) => [{ ts: "2026-09-05T23:59:00Z", type_: "Warning", reason: "BackOff", message: `${kind} warning`, involved_kind: kind, involved_name: name, involved_uid: `uid-${kind}-${name}`, count: 5 }, { ts: null, type_: "Warning", reason: "Unrelated", message: "other resource", involved_kind: "Service", involved_name: name, involved_uid: "other", count: 1 }, { ts: null, type_: "Normal", reason: "Pulled", message: "image already present", involved_kind: kind, involved_name: name, involved_uid: `uid-${kind}-${name}`, count: 1 }]);
});
it("connects failing container, exact events, observed owners and scoped export", async () => {
  renderInvestigation();
  const previous = await screen.findByRole("link", { name: /previous logs.*worker/i });
  expect(previous).toHaveAttribute("href", "/cluster/prod/logs?ns=payments&kind=pod&name=api&c=worker&startedAt=2026-09-06T00%3A00%3A00Z&previous=true");
  expect(await screen.findByText(/deployment\/api · 0\/1/)).toBeInTheDocument();
  expect(k8s.listEventsFor).toHaveBeenCalledWith("payments", "pod", "api", "prod");
  expect(k8s.getResource).toHaveBeenCalledWith("payments", "deployment", "api", "prod");
  expect(screen.queryByText("other resource")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  const report = screen.getByTestId("report");
  expect(report).toHaveTextContent("payments");
  expect(report).toHaveTextContent("2026-09-06T00:00:00Z");
  expect(report).toHaveTextContent("deployment/api · 0/1");
  expect(report).toHaveTextContent("Events without matching involved-object UID");
  expect(report).toHaveTextContent("no rollout notes available");
  expect(report).not.toHaveTextContent("other resource");
  expect(report).toHaveTextContent("image already present");
});
it("shows denied sources explicitly while allowing a partial scoped report", async () => {
  vi.mocked(k8s.getResource).mockRejectedValue(new Error("forbidden"));
  vi.mocked(k8s.listEventsFor).mockRejectedValue(new Error("forbidden"));
  renderInvestigation();
  expect(await screen.findByText(/Resource: unavailable/i)).toBeInTheDocument();
  expect(screen.getByText(/Related events: unavailable/i)).toBeInTheDocument();
  expect(screen.getByText(/Owner rollout evidence unavailable/i)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  expect(screen.getByTestId("report")).toHaveTextContent("unavailable");
});
it("does not expose old target results after context and selection change", async () => {
  let complete!: (value: ResourceDetail) => void;
  vi.mocked(k8s.getResource).mockImplementation((_ns, kind, name, ctx) => ctx === "prod" ? new Promise((resolve) => { complete = resolve; }) : Promise.resolve(detail(kind, name)));
  const view = renderInvestigation();
  await waitFor(() => expect(complete).toBeDefined());
  view.rerender(<TriageInvestigation context="stage" issue={{ ...issue, resource: { ...issue.resource, name: "other" } }} startedAt="2026-09-06T01:00:00Z" onClose={() => {}} />);
  complete(detail("pod", "api", [{ kind: "Deployment", name: "stale-owner" }]));
  await screen.findByText(/stage · payments · pod\/other/i);
  expect(screen.queryByText(/stale-owner/)).not.toBeInTheDocument();
  expect(k8s.getResource).not.toHaveBeenCalledWith("payments", "deployment", "stale-owner", "stage");
});

it("redacts credential-like owner event messages before display and export", async () => {
  vi.mocked(k8s.listEventsFor).mockImplementation(async (_ns, kind, name) => [{ ts: null, type_: "Warning", reason: "Failed", message: "token: super-secret-owner-event", involved_kind: kind, involved_name: name, involved_uid: `uid-${kind}-${name}`, count: 1 }]);
  renderInvestigation();
  await screen.findByText(/deployment\/api · 0\/1/);
  await waitFor(() => expect(screen.getByRole("button", { name: /export investigation/i })).not.toBeDisabled());
  expect(screen.queryByText(/super-secret-owner-event/)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  expect(screen.getByTestId("report")).not.toHaveTextContent("super-secret-owner-event");
});


type NativeCapture = { text: string; truncated: boolean; bytes: number };
function deferredCapture() {
  let resolve!: (value: NativeCapture) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<NativeCapture>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const captured = (text: string): NativeCapture => ({ text, truncated: false, bytes: new TextEncoder().encode(text).length });
async function readyCapture() {
  await screen.findByRole("link", { name: /previous logs.*worker/i });
  await waitFor(() => expect(screen.getByRole("button", { name: /export investigation/i })).toBeEnabled());
}
function reportExcerpt() {
  const markdown = screen.getByTestId("report").textContent ?? "";
  const match = markdown.match(/```text\n([\s\S]*?)\n```/);
  expect(match).not.toBeNull();
  return match![1];
}
it("captures through native IPC, previews redacted text, and excludes unchecked evidence from export", async () => {
  vi.mocked(k8s.captureIncidentLogs).mockResolvedValue(captured("safe log marker\nAuthorization: Bearer synthetic-ui-secret"));
  renderInvestigation(); await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(await screen.findByLabelText("Log evidence preview")).toHaveValue("safe log marker\nAuthorization: Bearer [REDACTED]");
  expect(k8s.captureIncidentLogs).toHaveBeenCalledWith("prod", "payments", "api", "uid-pod-api", "worker", false);
  expect(document.body.textContent).not.toContain("synthetic-ui-secret");
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  expect(reportExcerpt()).toContain("safe log marker");
  expect(screen.getByTestId("report")).not.toHaveTextContent("synthetic-ui-secret");
  await userEvent.click(screen.getByLabelText("include edited excerpt in export"));
  expect(screen.getByTestId("report")).not.toHaveTextContent("safe log marker");
  expect(screen.getByTestId("report")).not.toHaveTextContent("Authorization");
});
it.each([
  ["short bearer expansion", "Bearer a ".repeat(4000)],
  ["ordinary oversized paste", "z".repeat(25000)],
  ["line limit paste", Array.from({ length: 250 }, (_, i) => `log line ${i}`).join("\n")],
])("bounds edited preview and final exported excerpt: %s", async (_label, text) => {
  vi.mocked(k8s.captureIncidentLogs).mockResolvedValue(captured("seed log"));
  renderInvestigation(); await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  const preview = await screen.findByLabelText("Log evidence preview") as HTMLTextAreaElement;
  fireEvent.change(preview, { target: { value: text } });
  expect(preview.value.length).toBeLessThanOrEqual(20000);
  expect(preview.value.split("\n").length).toBeLessThanOrEqual(200);
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  const excerpt = reportExcerpt();
  expect(excerpt.length).toBeLessThanOrEqual(20000);
  expect(excerpt.split("\n").length).toBeLessThanOrEqual(200);
  expect(excerpt).not.toMatch(/Bearer a(?: |$)/);
  expect(screen.getByTestId("report")).toHaveTextContent("truncated");
});
it.each(["container", "previous", "current"])("keeps capture B busy when stale A completes after %s selection changes", async (selection) => {
  const a = deferredCapture(), b = deferredCapture();
  vi.mocked(k8s.captureIncidentLogs).mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
  renderInvestigation(); await readyCapture();
  if (selection === "current") await userEvent.click(screen.getByLabelText("capture previous instance"));
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(k8s.captureIncidentLogs).toHaveBeenNthCalledWith(1, "prod", "payments", "api", "uid-pod-api", "worker", selection === "current");
  expect(screen.getByRole("button", { name: "capturing…" })).toBeDisabled();
  if (selection === "container") await userEvent.selectOptions(screen.getByLabelText("Investigation container"), "sidecar");
  else await userEvent.click(screen.getByLabelText("capture previous instance"));
  expect(screen.getByRole("button", { name: "capture bounded logs" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(k8s.captureIncidentLogs).toHaveBeenNthCalledWith(2, "prod", "payments", "api", "uid-pod-api", selection === "container" ? "sidecar" : "worker", selection === "previous");
  await act(async () => { a.resolve(captured("STALE_CAPTURE_A")); });
  expect(screen.getByRole("button", { name: "capturing…" })).toBeDisabled();
  expect(screen.queryByLabelText("Log evidence preview")).not.toBeInTheDocument();
  await act(async () => { b.resolve(captured("CURRENT_CAPTURE_B")); });
  expect(await screen.findByLabelText("Log evidence preview")).toHaveValue("CURRENT_CAPTURE_B");
  expect(screen.getByRole("button", { name: "capture bounded logs" })).toBeEnabled();
  expect(document.body.textContent).not.toContain("STALE_CAPTURE_A");
});
it("recovers capture after a safe native failure and distinguishes successful empty EOF", async () => {
  vi.mocked(k8s.captureIncidentLogs).mockRejectedValueOnce(new Error("timeout token=synthetic-error-secret")).mockResolvedValueOnce(captured(""));
  renderInvestigation(); await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  await screen.findByText(/Log capture failed, timed out/);
  expect(screen.queryByLabelText("Log evidence preview")).not.toBeInTheDocument();
  expect(document.body.textContent).not.toContain("synthetic-error-secret");
  expect(screen.getByRole("button", { name: "capture bounded logs" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(await screen.findByLabelText("Log evidence preview")).toHaveValue("");
  expect(screen.getByLabelText("include edited excerpt in export")).toBeChecked();
});
it("ignores a late capture from an old target while the new target capture is pending", async () => {
  const a = deferredCapture(), b = deferredCapture();
  vi.mocked(k8s.captureIncidentLogs).mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
  const view = renderInvestigation(); await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  view.rerender(<TriageInvestigation context="stage" issue={{ ...issue, resource: { ...issue.resource, name: "other" } }} startedAt="2026-09-06T01:00:00Z" onClose={() => {}} />);
  await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(k8s.captureIncidentLogs).toHaveBeenNthCalledWith(2, "stage", "payments", "other", "uid-pod-other", "worker", false);
  await act(async () => { a.resolve(captured("OLD_TARGET_LOG")); });
  expect(screen.getByRole("button", { name: "capturing…" })).toBeDisabled();
  expect(screen.queryByLabelText("Log evidence preview")).not.toBeInTheDocument();
  await act(async () => { b.resolve(captured("NEW_TARGET_LOG")); });
  expect(await screen.findByLabelText("Log evidence preview")).toHaveValue("NEW_TARGET_LOG");
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  expect(reportExcerpt()).toBe("NEW_TARGET_LOG");
  expect(screen.getByTestId("report")).not.toHaveTextContent("OLD_TARGET_LOG");
});

it("invalidates pending capture when observed pod UID changes before starting the replacement capture", async () => {
  const a = deferredCapture(), b = deferredCapture();
  vi.mocked(k8s.captureIncidentLogs).mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
  const { client } = renderInvestigation(); await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  await act(async () => { client.setQueryData(["investigation-resource", "prod", "payments", "pod", "api", "2026-09-06T00:00:00Z"], { ...detail("pod", "api"), yaml: "metadata:\n  uid: replacement-uid" }); });
  await waitFor(() => expect(screen.getByRole("button", { name: "capture bounded logs" })).toBeEnabled());
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(k8s.captureIncidentLogs).toHaveBeenNthCalledWith(2, "prod", "payments", "api", "replacement-uid", "worker", false);
  await act(async () => { a.resolve(captured("OLD_UID_LOG")); });
  expect(screen.getByRole("button", { name: "capturing…" })).toBeDisabled();
  await act(async () => { b.resolve(captured("NEW_UID_LOG")); });
  expect(await screen.findByLabelText("Log evidence preview")).toHaveValue("NEW_UID_LOG");
});

it("invalidates old preview when query data changes the automatically suggested container", async () => {
  vi.mocked(k8s.captureIncidentLogs).mockResolvedValue(captured("WORKER_LOG"));
  const { client } = renderInvestigation(); await readyCapture();
  await userEvent.click(screen.getByRole("button", { name: "capture bounded logs" }));
  expect(await screen.findByLabelText("Log evidence preview")).toHaveValue("WORKER_LOG");
  await act(async () => { client.setQueryData(["investigation-pod", "prod", "payments", "pod", "api", "2026-09-06T00:00:00Z"], { name: "api", namespace: "payments", containers: [{ name: "sidecar", ready: false, restart_count: 10, state: "waiting" }] }); });
  await waitFor(() => expect(screen.queryByLabelText("Log evidence preview")).not.toBeInTheDocument());
  expect(screen.getByLabelText("Investigation container")).toHaveValue("sidecar");
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  expect(screen.getByTestId("report")).not.toHaveTextContent("WORKER_LOG");
});

it("exports current scheduling evidence but excludes cached evidence after failed refresh or metadata refresh", async () => {
  const { client } = renderInvestigation(); await readyCapture();
  await screen.findByText(/CURRENT_GPU_EVIDENCE/);
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  expect(screen.getByTestId("report")).toHaveTextContent("CURRENT_GPU_EVIDENCE");
  expect(screen.getByTestId("report")).toHaveTextContent("nodes: forbidden · incomplete");
  expect(screen.getByTestId("report")).toHaveTextContent("scheduling · observed · captured 2026-10-02T00:00:00Z");
  expect(screen.getByTestId("report")).not.toHaveTextContent("synthetic-gpu-secret");
  vi.mocked(fetchGpuScheduling).mockRejectedValue(new Error("forbidden"));
  await userEvent.click(screen.getByRole("button", { name: /refresh scheduling/i }));
  await screen.findByText(/Scheduling evidence unavailable/);
  expect(screen.getByTestId("report")).not.toHaveTextContent("CURRENT_GPU_EVIDENCE");
  vi.mocked(k8s.getResource).mockRejectedValue(new Error("forbidden"));
  await act(async () => { await client.invalidateQueries({ queryKey: ["investigation-resource"] }); });
  expect(screen.getByTestId("report")).not.toHaveTextContent("CURRENT_GPU_EVIDENCE");
});
it("invalidates old scheduling explanations when metadata observes a replacement UID", async () => {
  const { client } = renderInvestigation(); await readyCapture();
  await screen.findByText(/CURRENT_GPU_EVIDENCE/);
  await userEvent.click(screen.getByRole("button", { name: /export investigation/i }));
  let resolve!: (value: Awaited<ReturnType<typeof fetchGpuScheduling>>) => void;
  vi.mocked(fetchGpuScheduling).mockReturnValueOnce(new Promise((yes) => { resolve = yes; }));
  await act(async () => { client.setQueryData(["investigation-resource", "prod", "payments", "pod", "api", "2026-09-06T00:00:00Z"], { ...detail("pod", "api"), yaml: "metadata:\n  uid: replacement-uid" }); });
  await waitFor(() => expect(screen.getByTestId("report")).not.toHaveTextContent("CURRENT_GPU_EVIDENCE"));
  await waitFor(() => expect(fetchGpuScheduling).toHaveBeenLastCalledWith("prod", "payments", "api", "replacement-uid"));
  await act(async () => { resolve({ pod: { api_version: "v1", kind: "Pod", namespace: "payments", name: "api", uid: "replacement-uid" }, sources: {}, explanations: [] }); });
  expect(screen.getByTestId("report")).not.toHaveTextContent("CURRENT_GPU_EVIDENCE");
});
