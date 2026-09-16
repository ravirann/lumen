import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { k8s } from "./k8s";
import { customResources } from "./customResources";
import { useUiSettings } from "@/state/uiSettings";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
beforeEach(() => { vi.clearAllMocks(); useUiSettings.setState({ readOnly: false }); });
it("blocks every mutation family before dispatch to a protected context", async () => {
  vi.mocked(invoke).mockResolvedValue({ context: "prod", protected: true, unlocked_until_ms: null, can_mutate: false });
  const calls = [
    () => customResources.write({ crd_name: "widgets.example.io", version: "v1", namespace: "team", name: "sample" }, "", true, false, "prod"),
    () => customResources.delete({ crd_name: "widgets.example.io", version: "v1", namespace: "team", name: "sample", uid: "uid" }, "prod"),
    () => k8s.restartWorkload("ns", "deployment", "api", "prod"),
    () => k8s.deleteResource("ns", "pod", "api", "prod"),
    () => k8s.cordonNode("worker", "prod"),
    () => k8s.applyResource("ns", "deployment", "api", "", false, "prod"),
    () => k8s.provisionTeamAccess({} as any, "prod"),
    () => k8s.renewTeamToken("member", 1, "prod"),
    () => k8s.helmInstall({} as any, "id", {} as any, "prod"),
    () => k8s.syncArgocdApplication("prod", "ns", "api", {}),
    () => k8s.refreshArgocdApplication("prod", "ns", "api", false),
    () => k8s.cancelPipelineRun("prod", "ns", "run"),
    () => k8s.createDebugContainer({ context: "prod" } as any),
    () => k8s.startPodAttach({} as any, {} as any, "prod"),
  ];
  for (const call of calls) await expect(call()).rejects.toThrow(/locked/);
  expect(vi.mocked(invoke).mock.calls.every(([command]) => command === "get_context_protection")).toBe(true);
});
it("preserves the explicit dev target even if UI focus changes while native status is loading", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_context_protection" ? { context: "dev", protected: false, unlocked_until_ms: null, can_mutate: true } : undefined);
  await k8s.cordonNode("worker", "dev");
  expect(invoke).toHaveBeenLastCalledWith("cordon_node", { name: "worker", context: "dev" });
});
