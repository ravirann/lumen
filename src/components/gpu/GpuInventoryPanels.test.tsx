import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, expect, it, vi } from "vitest";
import { GpuInventoryPanels } from "./GpuInventoryPanels";
import {
  fetchGpuInventory,
  type GpuInventorySnapshot,
} from "@/lib/gpuInventory";
vi.mock("@/lib/gpuInventory", async (original) => ({
  ...(await original<typeof import("@/lib/gpuInventory")>()),
  fetchGpuInventory: vi.fn(),
}));
const snapshot = (): GpuInventorySnapshot => ({
  namespace: "team",
  captured_at: "2026-10-02T00:00:00Z",
  pods: {
    state: "available",
    complete: false,
    captured_at: "2026-10-02T00:00:00Z",
    message: null,
    items: [
      {
        resource: {
          api_version: "v1",
          kind: "Pod",
          name: "train",
          namespace: "team",
          uid: "u",
        },
        phase: "Running",
        node_name: "worker",
        owners: [],
        requests: { "nvidia.com/gpu": null, "nvidia.com/gpu.shared": 8 },
      },
    ],
  },
  nodes: {
    state: "forbidden",
    complete: false,
    captured_at: "2026-10-02T00:00:00Z",
    message: null,
    items: [],
  },
});
function wrapper({ children }: { children: React.ReactNode }) {
  return (
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <MemoryRouter>{children}</MemoryRouter>
    </QueryClientProvider>
  );
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(fetchGpuInventory).mockResolvedValue(snapshot());
});
it("shows partial namespace totals, unknown quantities and denied nodes independently", async () => {
  render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
  expect(
    await screen.findByText("Observed namespace requests"),
  ).toBeInTheDocument();
  expect(screen.getByText(/GPU workloads: partial/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "train" })).toHaveAttribute(
    "href",
    "/cluster/demo/workloads/pods?ns=team&q=train",
  );
  expect(
    screen.getByRole("link", { name: "DRA allocations for train" }),
  ).toHaveAttribute("href", "/cluster/demo/device-resources?ns=team&pod=train");
  expect(screen.getAllByText("Unknown").length).toBeGreaterThan(0);
  await userEvent.click(screen.getByRole("tab", { name: "GPU nodes" }));
  expect(screen.getByText(/GPU nodes: access denied/)).toBeInTheDocument();
});
it("supports arrow-key tabs, advertised slots and metadata", async () => {
  const s = snapshot();
  s.nodes = {
    ...s.nodes,
    state: "available",
    complete: true,
    items: [
      {
        resource: {
          api_version: "v1",
          kind: "Node",
          namespace: null,
          name: "worker",
          uid: "n",
        },
        allocatable: { "nvidia.com/gpu.shared": 16 },
        gpu_labels: { "nvidia.com/gpu.product": "A100" },
      },
    ],
  };
  vi.mocked(fetchGpuInventory).mockResolvedValue(s);
  render(<GpuInventoryPanels context="demo" namespace="" />, { wrapper });
  await screen.findByText("Observed requests");
  screen.getByRole("tab", { name: "GPU workloads" }).focus();
  await userEvent.keyboard("{ArrowRight}");
  expect(screen.getByRole("tab", { name: "GPU nodes" })).toHaveFocus();
  expect(screen.getByText("Advertised slots")).toBeInTheDocument();
  expect(screen.getByText(/A100/)).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "worker · DRA device details" }),
  ).toHaveAttribute("href", "/cluster/demo/device-resources?ns=&node=worker");
});
it("distinguishes empty, loading and failed sources", async () => {
  const s = snapshot();
  s.pods.items = [];
  s.pods.complete = true;
  vi.mocked(fetchGpuInventory).mockResolvedValue(s);
  render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
  expect(screen.getByText("Loading GPU inventory…")).toBeInTheDocument();
  expect(
    await screen.findByText("No GPU requests observed in this scope."),
  ).toBeInTheDocument();
});
it("discards late context and namespace results", async () => {
  let resolve!: (s: GpuInventorySnapshot) => void;
  vi.mocked(fetchGpuInventory)
    .mockImplementationOnce(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    )
    .mockImplementation(() => new Promise(() => {}));
  const view = render(<GpuInventoryPanels context="demo" namespace="team" />, {
    wrapper,
  });
  view.rerender(<GpuInventoryPanels context="other" namespace="other" />);
  await act(async () => resolve(snapshot()));
  expect(screen.queryByText("train")).not.toBeInTheDocument();
  expect(screen.getByText("Loading GPU inventory…")).toBeInTheDocument();
});

it("does not render denied source items or call missing coverage zero", async () => {
  const s = snapshot();
  s.pods.state = "forbidden";
  vi.mocked(fetchGpuInventory).mockResolvedValue(s);
  render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
  expect(
    await screen.findByText(/GPU workloads: access denied/),
  ).toBeInTheDocument();
  expect(screen.queryByText("train")).not.toBeInTheDocument();
  expect(
    screen.queryByText("No GPU requests observed in this scope."),
  ).not.toBeInTheDocument();
});
it("clears previous data on failed refresh and provides retry", async () => {
  vi.mocked(fetchGpuInventory)
    .mockResolvedValueOnce(snapshot())
    .mockRejectedValueOnce(new Error("secret native error"));
  render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
  await screen.findByText("train");
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh GPU inventory" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not load GPU inventory",
  );
  expect(screen.queryByText("train")).not.toBeInTheDocument();
  expect(screen.queryByText(/secret native error/)).not.toBeInTheDocument();
});
it("uses narrow-layout table scrolling and bounds rendered rows", async () => {
  const s = snapshot();
  s.pods.items = Array.from({ length: 101 }, (_, i) => ({
    ...s.pods.items[0],
    resource: {
      ...s.pods.items[0].resource,
      name: `train-${i}`,
      uid: `u-${i}`,
    },
  }));
  vi.mocked(fetchGpuInventory).mockResolvedValue(s);
  render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
  await screen.findByText("train-0");
  expect(screen.getByRole("table").parentElement).toHaveClass(
    "overflow-x-auto",
  );
  expect(screen.getAllByRole("link", { name: /^train-/ })).toHaveLength(50);
  await userEvent.click(
    screen.getByRole("button", { name: "Next GPU workloads page" }),
  );
  expect(screen.getByText("train-50")).toBeInTheDocument();
});

it.each([
  ["unsafe integer", Number.MAX_SAFE_INTEGER + 1],
  ["fractional", 1.5],
  ["negative", -1],
  ["infinite", Infinity],
  ["malformed", "invalid" as unknown as number],
])(
  "keeps %s quantities unknown in pod and node rows",
  async (_label, quantity) => {
    const s = snapshot();
    s.pods.items[0].requests = { "nvidia.com/gpu": quantity };
    s.nodes = {
      ...s.nodes,
      state: "available",
      complete: true,
      items: [
        {
          resource: {
            api_version: "v1",
            kind: "Node",
            namespace: null,
            name: "worker",
            uid: "n",
          },
          allocatable: { "nvidia.com/gpu": quantity },
          gpu_labels: {},
        },
      ],
    };
    vi.mocked(fetchGpuInventory).mockResolvedValue(s);
    render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
    const pod = await screen.findByRole("link", { name: "train" });
    expect(within(pod.closest("tr")!).getByText("Unknown")).toBeInTheDocument();
    expect(screen.getAllByText("Unknown")).toHaveLength(2);
    await userEvent.click(screen.getByRole("tab", { name: "GPU nodes" }));
    const node = screen.getByRole("link", {
      name: "worker · DRA device details",
    });
    expect(
      within(node.closest("tr")!).getByText("Unknown"),
    ).toBeInTheDocument();
  },
);
it("never renders rounded native i64 node slots as definite evidence", async () => {
  const s = snapshot();
  s.nodes = {
    ...s.nodes,
    state: "available",
    complete: true,
    items: [
      {
        resource: {
          api_version: "v1",
          kind: "Node",
          namespace: null,
          name: "worker",
          uid: "n",
        },
        allocatable: { "nvidia.com/gpu": Number.MAX_SAFE_INTEGER + 1 },
        gpu_labels: {},
      },
    ],
  };
  vi.mocked(fetchGpuInventory).mockResolvedValue(s);
  render(<GpuInventoryPanels context="demo" namespace="team" />, { wrapper });
  await screen.findByText("train");
  await userEvent.click(screen.getByRole("tab", { name: "GPU nodes" }));
  expect(
    within(
      screen
        .getByRole("link", { name: "worker · DRA device details" })
        .closest("tr")!,
    ).getByText("Unknown"),
  ).toBeInTheDocument();
});
