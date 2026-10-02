import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { SchedulingSnapshot } from "@/lib/gpuTypes";
import { GpuSchedulingPanel } from "./GpuSchedulingPanel";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const snapshot = (uid = "u1", message = "Insufficient nvidia.com/gpu"): SchedulingSnapshot => ({
  pod: { api_version: "v1", kind: "Pod", namespace: "team", name: "train", uid },
  sources: { nodes: { state: "forbidden", complete: false, captured_at: "2026-10-02T00:00:00Z", items: [], message: "Node access denied" } },
  explanations: ["admission", "scheduling", "startup"].map((stage, i) => ({ stage: stage as "admission" | "scheduling" | "startup", confidence: (["unknown", "observed", "inferred"] as const)[i], message: i === 1 ? message : `${stage} evidence`, sources: [], captured_at: "2026-10-02T00:00:00Z", transition_time: null, observed_generation: null })),
});
function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const onEvidence = vi.fn();
  const props = { context: "prod", namespace: "team", pod: "train", expectedUid: "u1", onEvidence };
  const Wrapper = ({ children }: { children: React.ReactNode }) => <QueryClientProvider client={client}><MemoryRouter>{children}</MemoryRouter></QueryClientProvider>;
  return { ...render(<GpuSchedulingPanel {...props} />, { wrapper: Wrapper }), client, onEvidence, props };
}
beforeEach(() => { vi.resetAllMocks(); vi.mocked(invoke).mockResolvedValue(snapshot()); });
it("collects explicit identity through IPC and shows stage, confidence, time and denied sources", async () => {
  const view = setup();
  expect(await screen.findByText("Insufficient nvidia.com/gpu")).toBeInTheDocument();
  expect(invoke).toHaveBeenCalledWith("gpu_scheduling_snapshot", { context: "prod", namespace: "team", pod: "train", expectedUid: "u1" });
  for (const stage of ["Admission", "Scheduling", "Startup"]) expect(screen.getByRole("heading", { name: stage })).toBeInTheDocument();
  for (const confidence of ["observed", "inferred", "unknown"]) expect(screen.getByText(new RegExp(`^${confidence} · captured`))).toBeInTheDocument();
  expect(screen.getByText(/nodes: forbidden.*incomplete/)).toBeInTheDocument();
  expect(screen.getAllByText(/2026-10-02T00:00:00Z/).length).toBeGreaterThan(0);
  await waitFor(() => expect(view.onEvidence).toHaveBeenLastCalledWith(snapshot().explanations));
});
it.each(["uid", "context"])("clears evidence and excludes late responses after %s changes", async (field) => {
  let resolve!: (value: SchedulingSnapshot) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise((yes) => { resolve = yes; })).mockResolvedValueOnce(snapshot(field === "uid" ? "u2" : "u1", "CURRENT"));
  const view = setup();
  view.rerender(<GpuSchedulingPanel {...view.props} context={field === "context" ? "stage" : "prod"} expectedUid={field === "uid" ? "u2" : "u1"} />);
  await screen.findByText("CURRENT");
  await act(async () => { resolve(snapshot("u1", "OLD")); });
  expect(screen.queryByText("OLD")).not.toBeInTheDocument();
  expect(JSON.stringify(view.onEvidence.mock.lastCall)).not.toContain("OLD");
});
it("excludes cached evidence while refreshing and after failure", async () => {
  const view = setup(); await screen.findByText("Insufficient nvidia.com/gpu");
  let reject!: (error: Error) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise((_yes, no) => { reject = no; }));
  fireEvent.click(screen.getByRole("button", { name: /refresh scheduling/i }));
  await waitFor(() => expect(view.onEvidence).toHaveBeenLastCalledWith([]));
  expect(screen.queryByText("Insufficient nvidia.com/gpu")).not.toBeInTheDocument();
  await act(async () => { reject(new Error("token=secret")); });
  expect(await screen.findByText(/Scheduling evidence unavailable/)).toBeInTheDocument();
  expect(screen.queryByText("Insufficient nvidia.com/gpu")).not.toBeInTheDocument();
  expect(document.body.textContent).not.toContain("secret");
  expect(view.onEvidence).toHaveBeenLastCalledWith([]);
});
it("rejects successful responses for a different UID", async () => {
  vi.mocked(invoke).mockResolvedValue(snapshot("replacement", "WRONG UID"));
  const view = setup();
  expect(await screen.findByText(/identity changed/)).toBeInTheDocument();
  expect(screen.queryByText("WRONG UID")).not.toBeInTheDocument();
  expect(view.onEvidence).toHaveBeenLastCalledWith([]);
});

it("keeps source links scoped and renders a successful empty capture explicitly", async () => {
  const result = snapshot();
  result.explanations[1].sources = [{ api_version: "v1", kind: "Pod", namespace: "team", name: "train", uid: "u1" }];
  vi.mocked(invoke).mockResolvedValueOnce(result).mockResolvedValueOnce({ ...result, explanations: [] });
  setup();
  const link = await screen.findByRole("link", { name: "Pod/train" });
  expect(link).toHaveAttribute("href", "/cluster/prod/workloads/pods?ns=team&q=train");
  fireEvent.click(screen.getByRole("button", { name: /refresh scheduling/i }));
  expect(await screen.findByText(/No current scheduling blocker established/)).toBeInTheDocument();
});
it("requires a selected UID and namespace before collecting evidence", async () => {
  const view = setup(); await screen.findByText("Insufficient nvidia.com/gpu");
  view.rerender(<GpuSchedulingPanel {...view.props} expectedUid="" />);
  expect(screen.getByText(/namespace and current pod UID are required/)).toBeInTheDocument();
  expect(view.onEvidence).toHaveBeenLastCalledWith([]);
  expect(invoke).toHaveBeenCalledTimes(1);
});
