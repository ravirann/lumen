import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useNavigate } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DeviceResourcesView } from "./DeviceResourcesView";
import {
  fetchDeviceResources,
  type DeviceSnapshot,
} from "@/lib/deviceResources";
import { fetchGpuInventory } from "@/lib/gpuInventory";
vi.mock("@/lib/gpuInventory", async (original) => ({
  ...(await original<typeof import("@/lib/gpuInventory")>()),
  fetchGpuInventory: vi.fn(),
}));
import { k8s } from "@/lib/k8s";
vi.mock("@/lib/deviceResources", async (original) => ({
  ...(await original<typeof import("@/lib/deviceResources")>()),
  fetchDeviceResources: vi.fn(),
}));
vi.mock("@/lib/k8s", () => ({
  k8s: { listNamespaces: vi.fn(), listContexts: vi.fn() },
}));
const available = () => ({
  state: "available" as const,
  items: [],
  message: null,
});
function snapshot(): DeviceSnapshot {
  return {
    namespace: "team",
    captured_at: "2026-09-10T00:00:00Z",
    claims: available(),
    templates: available(),
    classes: available(),
    slices: available(),
    pods: available(),
  };
}
function Navigation() {
  const navigate = useNavigate();
  return (
    <>
      <button
        onClick={() => navigate("/cluster/other/device-resources?ns=other")}
      >
        Switch scope
      </button>
      <DeviceResourcesView />
    </>
  );
}
function renderView(path = "/cluster/demo/device-resources?ns=team") {
  return render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route
            path="/cluster/:ctx/device-resources"
            element={<Navigation />}
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(fetchGpuInventory).mockResolvedValue({
    namespace: "team",
    captured_at: "now",
    pods: {
      state: "available",
      complete: true,
      captured_at: "now",
      items: [],
      message: null,
    },
    nodes: {
      state: "available",
      complete: true,
      captured_at: "now",
      items: [],
      message: null,
    },
  });
  vi.mocked(k8s.listNamespaces).mockResolvedValue(["team", "other"]);
  vi.mocked(k8s.listContexts).mockResolvedValue([]);
  vi.mocked(fetchDeviceResources).mockResolvedValue(snapshot());
});
describe("DeviceResourcesView", () => {
  it("honors explicit all-namespace URLs and distinguishes empty lists", async () => {
    renderView("/cluster/demo/device-resources?ns=");
    expect(
      await screen.findByText("No claims found in this scope."),
    ).toBeInTheDocument();
    expect(fetchDeviceResources).toHaveBeenCalledWith("demo", "");
    expect(
      screen.getByRole("button", { name: "namespace: all namespaces" }),
    ).toBeInTheDocument();
  });
  it("distinguishes forbidden and unsupported sources while leaving readable resources usable", async () => {
    const s = snapshot();
    s.claims = {
      state: "forbidden",
      items: [],
      message: "Cannot list resourceclaims",
    };
    s.classes = { state: "unsupported", items: [], message: null };
    s.templates.items = [
      { metadata: { name: "gpu-template", namespace: "team" } },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView();
    expect(
      await screen.findByText("Claims: access denied by cluster permissions."),
    ).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^Classes/ }));
    expect(
      screen.getByText("Classes: API not served by this cluster."),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Templates/ }));
    expect(screen.getByText("gpu-template")).toBeInTheDocument();
  });
  it("retries a failed snapshot explicitly", async () => {
    vi.mocked(fetchDeviceResources)
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue(snapshot());
    renderView();
    expect(await screen.findByRole("alert")).toHaveTextContent("offline");
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(
      await screen.findByText("No claims found in this scope."),
    ).toBeInTheDocument();
    expect(fetchDeviceResources).toHaveBeenCalledTimes(2);
  });
  it("does not display old scope data or an open inspector after navigation", async () => {
    const s = snapshot();
    s.claims.items = [
      { metadata: { name: "private-claim", namespace: "team" } },
    ];
    vi.mocked(fetchDeviceResources)
      .mockResolvedValueOnce(s)
      .mockImplementation(() => new Promise(() => {}));
    renderView();
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Inspect ResourceClaim private-claim",
      }),
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(screen.getByRole("button", { name: "Switch scope" }));
    await waitFor(() =>
      expect(fetchDeviceResources).toHaveBeenLastCalledWith("other", "other"),
    );
    expect(screen.queryByText("private-claim")).not.toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("Loading device resources…")).toBeInTheDocument();
  });
  it("redacts resource inspection and exposes no mutation controls", async () => {
    const s = snapshot();
    s.claims.items = [
      {
        metadata: {
          name: "gpu",
          namespace: "team",
          annotations: { secret: "never-render" },
        },
        spec: {
          devices: {
            config: [{ opaque: { parameters: { token: "never-render" } } }],
          },
        },
      },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView();
    await userEvent.click(
      await screen.findByRole("button", { name: "Inspect ResourceClaim gpu" }),
    );
    expect(screen.getByRole("dialog")).not.toHaveTextContent("never-render");
    expect(screen.getByRole("dialog")).toHaveTextContent("[redacted]");
    expect(
      screen.queryByRole("button", { name: /apply|delete|save/i }),
    ).not.toBeInTheDocument();
  });
  it("opens pod deep links and reports missing health as unknown", async () => {
    const s = snapshot();
    s.claims.items = [
      {
        metadata: { name: "gpu", namespace: "team" },
        status: {
          allocation: {
            devices: {
              results: [
                {
                  request: "request",
                  driver: "example.io",
                  pool: "pool",
                  device: "gpu0",
                },
              ],
            },
          },
        },
      },
    ];
    s.pods.items = [
      {
        metadata: { name: "training", namespace: "team" },
        spec: { resourceClaims: [{ name: "gpu", resourceClaimName: "gpu" }] },
      },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView("/cluster/demo/device-resources?ns=team&pod=training");
    expect(
      await screen.findByText("Health unknown · no matching container report"),
    ).toBeInTheDocument();
    expect(screen.getByText("example.io / pool / gpu0")).toBeInTheDocument();
  });
  it("joins a pod allocation to its current inventory slice and reports pool completeness", async () => {
    const s = snapshot();
    s.claims.items = [
      {
        metadata: { name: "gpu", namespace: "team" },
        status: {
          allocation: {
            devices: {
              results: [
                {
                  request: "request",
                  driver: "example.io",
                  pool: "pool",
                  device: "gpu0",
                },
              ],
            },
          },
        },
      },
    ];
    s.pods.items = [
      {
        metadata: { name: "training", namespace: "team" },
        spec: { resourceClaims: [{ name: "gpu", resourceClaimName: "gpu" }] },
      },
    ];
    s.slices.items = [
      {
        metadata: { name: "inventory-a" },
        spec: {
          driver: "example.io",
          pool: { name: "pool", generation: 2, resourceSliceCount: 2 },
          nodeName: "worker-a",
          devices: [{ name: "gpu0" }],
        },
      },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView("/cluster/demo/device-resources?ns=team&pod=training");
    expect(
      await screen.findByText(
        "Slice: inventory-a · worker-a · generation 2 · incomplete pool",
      ),
    ).toBeInTheDocument();
  });
  it("shows node selector access as unknown and never calls inventory free capacity", async () => {
    const s = snapshot();
    s.slices.items = [
      {
        metadata: { name: "inventory-a" },
        spec: {
          driver: "example.io",
          pool: { name: "pool", generation: 2, resourceSliceCount: 1 },
          nodeSelector: {
            nodeSelectorTerms: [
              { matchExpressions: [{ key: "gpu", operator: "Exists" }] },
            ],
          },
          devices: [{ name: "gpu0" }],
        },
      },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView("/cluster/demo/device-resources?ns=&node=worker-a");
    expect(
      await screen.findByText("0 devices with confirmed node access"),
    ).toBeInTheDocument();
    expect(screen.getByText("Unknown")).toBeInTheDocument();
    expect(screen.getByText("Complete · 1/1 slices")).toBeInTheDocument();
    expect(
      screen.getByText(/Published inventory does not measure free capacity/),
    ).toBeInTheDocument();
  });
  it("paginates all resources and filters before paging, resetting on collection and scope changes", async () => {
    const s = snapshot();
    s.claims.items = Array.from({ length: 101 }, (_, i) => ({
      metadata: {
        name: `claim-${String(i).padStart(3, "0")}`,
        namespace: "team",
      },
    }));
    s.templates.items = [
      { metadata: { name: "one-template", namespace: "team" } },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView();
    await screen.findByText("claim-000");
    const names = new Set<string>();
    const user = userEvent.setup();
    for (let page = 0; page < 3; page++) {
      const buttons = screen.getAllByRole("button", {
        name: /^Inspect ResourceClaim /,
      });
      expect(buttons.length).toBeLessThanOrEqual(50);
      buttons.forEach((button) =>
        names.add(button.getAttribute("aria-label")!),
      );
      if (page < 2)
        await user.click(
          screen.getByRole("button", { name: "Next resources page" }),
        );
    }
    expect(names.size).toBe(101);
    expect(screen.getByText("101–101 of 101 resources")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Next resources page" }),
    ).toBeDisabled();
    await user.type(
      screen.getByRole("textbox", { name: "Filter device resources" }),
      "claim-000",
    );
    expect(screen.getByText("1–1 of 1 resources")).toBeInTheDocument();
    expect(screen.getByText("claim-000")).toBeInTheDocument();
    await user.clear(
      screen.getByRole("textbox", { name: "Filter device resources" }),
    );
    expect(screen.getByText("1–50 of 101 resources")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Next resources page" }),
    );
    await user.click(screen.getByRole("button", { name: /^Templates/ }));
    expect(screen.getByText("one-template")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Claims/ }));
    expect(screen.getByText("1–50 of 101 resources")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Next resources page" }),
    );
    await user.click(screen.getByRole("button", { name: "Switch scope" }));
    await waitFor(() =>
      expect(fetchDeviceResources).toHaveBeenLastCalledWith("other", "other"),
    );
    expect(
      await screen.findByText("1–50 of 101 resources"),
    ).toBeInTheDocument();
  });
  it("bounds node devices and pod links while keeping the final entries reachable", async () => {
    const s = snapshot();
    s.pods.items = Array.from({ length: 51 }, (_, i) => ({
      metadata: { name: `pod-${i}`, namespace: "team" },
      spec: { resourceClaims: [{ name: "gpu", resourceClaimName: "gpu" }] },
    }));
    s.slices.items = [
      {
        metadata: { name: "inventory-a" },
        spec: {
          driver: "example.io",
          pool: { name: "pool", generation: 2, resourceSliceCount: 1 },
          nodeName: "worker-a",
          devices: Array.from({ length: 51 }, (_, i) => ({ name: `gpu-${i}` })),
        },
      },
    ];
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView("/cluster/demo/device-resources?ns=team&node=worker-a");
    expect(await screen.findByText("1–50 of 51 devices")).toBeInTheDocument();
    expect(screen.getByText("1–50 of 51 pods")).toBeInTheDocument();
    expect(screen.queryByText("gpu-50")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: "team/pod-50" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Next devices page" }),
    );
    expect(screen.getByText("gpu-50")).toBeInTheDocument();
    expect(screen.queryByText("gpu-0")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Next pods page" }),
    );
    expect(
      screen.getByRole("link", { name: "team/pod-50" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: "team/pod-0" }),
    ).not.toBeInTheDocument();
  });
  it("clamps the page after a smaller snapshot refresh", async () => {
    const first = snapshot();
    first.claims.items = Array.from({ length: 51 }, (_, i) => ({
      metadata: { name: `claim-${i}`, namespace: "team" },
    }));
    const next = snapshot();
    next.claims.items = first.claims.items.slice(0, 1);
    vi.mocked(fetchDeviceResources)
      .mockResolvedValueOnce(first)
      .mockResolvedValue(next);
    renderView();
    await screen.findByText("claim-0");
    await userEvent.click(
      screen.getByRole("button", { name: "Next resources page" }),
    );
    expect(screen.getByText("claim-50")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("1–1 of 1 resources")).toBeInTheDocument();
    expect(screen.getByText("claim-0")).toBeInTheDocument();
  });
  it("never presents failed partial source items as a complete list", async () => {
    const s = snapshot();
    s.claims = {
      state: "error",
      items: [{ metadata: { name: "partial-claim", namespace: "team" } }],
      message: "Listing incomplete",
    };
    vi.mocked(fetchDeviceResources).mockResolvedValue(s);
    renderView();
    expect(
      await screen.findByText("Claims: could not load resources."),
    ).toBeInTheDocument();
    expect(screen.queryByText("partial-claim")).not.toBeInTheDocument();
    expect(
      screen.queryByText("No claims found in this scope."),
    ).not.toBeInTheDocument();
  });
});

it("keeps GPU workloads usable when DRA APIs are absent", async () => {
  const s = snapshot();
  s.claims = { state: "unsupported", items: [], message: null };
  vi.mocked(fetchDeviceResources).mockResolvedValue(s);
  renderView();
  expect(
    await screen.findByText("Claims: API not served by this cluster."),
  ).toBeInTheDocument();
  expect(
    await screen.findByText("No GPU requests observed in this scope."),
  ).toBeInTheDocument();
  expect(fetchGpuInventory).toHaveBeenCalledWith("demo", "team");
});
