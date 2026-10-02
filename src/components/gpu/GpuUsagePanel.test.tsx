import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { GpuUsagePanel } from "./GpuUsagePanel";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const config = {
  namespace: "metrics",
  service: "prom",
  port: "9090",
  cluster_label: null,
  cluster_value: null,
  single_cluster_acknowledged: true,
};
const history = {
  captured_at: "2026-10-02T00:00:00Z",
  complete: true,
  warnings: [],
  series: [
    {
      family: "utilization",
      labels: { UUID: "GPU-gone" },
      points: [
        [1, 0],
        [2, null],
        [3, 50],
      ],
    },
  ],
};
function mount(context = "demo") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <GpuUsagePanel context={context} namespace="team" />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "gpu_telemetry_config_get"
      ? config
      : command === "gpu_telemetry_capabilities"
        ? {
            allowed: true,
            message: null,
            available_families: ["utilization"],
            identity_labels: ["UUID"],
          }
        : command === "gpu_telemetry_history"
          ? history
          : Promise.reject(new Error("SECRET")),
  );
});
it("offers setup without fetching telemetry when unconfigured", async () => {
  vi.mocked(invoke).mockResolvedValue(null);
  mount();
  await screen.findByText("Configure existing Prometheus Service");
  expect(invoke).not.toHaveBeenCalledWith(
    "gpu_telemetry_history",
    expect.anything(),
  );
});
it("retains device history without live inventory and labels missing families and gaps", async () => {
  mount();
  await screen.findByText("GPU-gone");
  expect(screen.getByText(/Identity coverage: UUID/)).toBeInTheDocument();
  expect(screen.getByText(/Missing families:/)).toBeInTheDocument();
  expect(screen.getByText(/1 gap/)).toBeInTheDocument();
  expect(screen.getByText(/· device-only ·/)).toBeInTheDocument();
});
it("sanitizes denied proxy and does not fetch history", async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "gpu_telemetry_config_get"
      ? config
      : command === "gpu_telemetry_capabilities"
        ? {
            allowed: false,
            message: "SECRET",
            available_families: [],
            identity_labels: [],
          }
        : Promise.reject(new Error("SECRET")),
  );
  mount();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    /Service proxy access unavailable/,
  );
  expect(screen.queryByText(/SECRET/)).not.toBeInTheDocument();
  expect(
    vi.mocked(invoke).mock.calls.some(([c]) => c === "gpu_telemetry_history"),
  ).toBe(false);
});
it("hides previous results after failed explicit refresh and sanitizes bounds failure", async () => {
  mount();
  await screen.findByText("GPU-gone");
  vi.mocked(invoke).mockImplementation(async () => {
    throw new Error("SECRET oversized");
  });
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh history" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(/narrower window/);
  expect(screen.queryByText("GPU-gone")).not.toBeInTheDocument();
  expect(screen.queryByText(/SECRET/)).not.toBeInTheDocument();
});
it("makes no new requests after unmount", async () => {
  const view = mount();
  await screen.findByText("GPU-gone");
  view.unmount();
  const count = vi.mocked(invoke).mock.calls.length;
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 30));
  });
  expect(invoke).toHaveBeenCalledTimes(count);
});
it("discards an old context history response in flight", async () => {
  let resolve!: (v: typeof history) => void;
  vi.mocked(invoke).mockImplementation(async (command, args) =>
    command === "gpu_telemetry_config_get"
      ? config
      : command === "gpu_telemetry_capabilities"
        ? {
            allowed: true,
            message: null,
            available_families: ["utilization"],
            identity_labels: [],
          }
        : command === "gpu_telemetry_history"
          ? (args as { context: string }).context === "old"
            ? new Promise((r) => {
                resolve = r;
              })
            : { ...history, series: [] }
          : Promise.reject(new Error("denied")),
  );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <GpuUsagePanel context="old" namespace="team" />
    </QueryClientProvider>,
  );
  await screen.findByText("Loading historical readings…");
  view.rerender(
    <QueryClientProvider client={client}>
      <GpuUsagePanel context="new" namespace="team" />
    </QueryClientProvider>,
  );
  await screen.findByText("No readings returned for this window.");
  await act(async () => resolve(history));
  expect(screen.queryByText("GPU-gone")).not.toBeInTheDocument();
});
it("keeps ambiguous and shared associations conservative", async () => {
  const pod = {
    resource: {
      api_version: "v1",
      kind: "Pod",
      namespace: "team",
      name: "train",
      uid: "uid",
    },
    phase: "Running",
    node_name: "node",
    requests: { "nvidia.com/gpu.shared": 1 },
    owners: [],
  };
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "gpu_telemetry_config_get"
      ? config
      : command === "gpu_telemetry_capabilities"
        ? {
            allowed: true,
            message: null,
            available_families: ["utilization"],
            identity_labels: ["pod"],
          }
        : command === "gpu_telemetry_history"
          ? {
              ...history,
              series: [
                {
                  ...history.series[0],
                  labels: {
                    UUID: "GPU-gone",
                    namespace: "team",
                    pod: "train",
                    pod_uid: "uid",
                  },
                },
              ],
            }
          : command === "gpu_inventory_snapshot"
            ? {
                pods: { state: "available", items: [pod] },
                nodes: { state: "available", items: [] },
                namespace: "team",
              }
            : Promise.reject(new Error("no mapping")),
  );
  mount();
  await screen.findByText("GPU-gone");
  await userEvent.selectOptions(
    screen.getByLabelText("Current workload association"),
    "uid",
  );
  expect(screen.getByText(/· device-only ·/)).toBeInTheDocument();
});
it("discards pending history after removing or changing source", async () => {
  let saved = config;
  let resolve!: (v: typeof history) => void;
  vi.mocked(invoke).mockImplementation(async (command, args) =>
    command === "gpu_telemetry_config_get"
      ? saved
      : command === "gpu_telemetry_config_set"
        ? ((saved = (args as { config: typeof config }).config), null)
        : command === "gpu_telemetry_capabilities"
          ? {
              allowed: true,
              message: null,
              available_families: ["utilization"],
              identity_labels: [],
            }
          : command === "gpu_telemetry_history"
            ? new Promise((r) => {
                resolve = r;
              })
            : Promise.reject(new Error("no mapping")),
  );
  mount();
  await screen.findByText("Loading historical readings…");
  await userEvent.click(screen.getByRole("button", { name: "Remove source" }));
  await act(async () => resolve(history));
  expect(screen.queryByText("GPU-gone")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Refresh history" }),
  ).not.toBeInTheDocument();
});

it("selects DRA-only empty-request consumers and verifies mapping end to end", async () => {
  const source = (items: Record<string, unknown>[]) => ({
    state: "available",
    items,
    message: null,
  });
  const slice = {
    spec: {
      driver: "gpu.nvidia.com",
      nodeName: "node",
      pool: { name: "pool" },
      devices: [
        {
          name: "one",
          attributes: { type: { string: "gpu" }, uuid: { string: "GPU-gone" } },
        },
      ],
    },
  };
  let ambiguous = false;
  const snapshot = () => ({
    namespace: "team",
    captured_at: "mapping-now",
    templates: source([]),
    classes: source([]),
    pods: source([
      {
        metadata: { namespace: "team", name: "train", uid: "uid" },
        spec: {
          nodeName: "node",
          resourceClaims: [{ name: "gpu-claim", resourceClaimName: "claim" }],
          containers: [
            {
              name: "worker",
              resources: { claims: [{ name: "gpu-claim", request: "req" }] },
            },
          ],
        },
      },
    ]),
    claims: source([
      {
        metadata: { namespace: "team", name: "claim" },
        status: {
          reservedFor: [{ resource: "pods", name: "train", uid: "uid" }],
          allocation: {
            devices: {
              results: [
                {
                  driver: "gpu.nvidia.com",
                  pool: "pool",
                  device: "one",
                  request: "req",
                },
              ],
            },
          },
        },
      },
    ]),
    slices: source(ambiguous ? [slice, slice] : [slice]),
  });
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "gpu_telemetry_config_get"
      ? {
          ...config,
          cluster_label: "cluster",
          cluster_value: "demo",
          single_cluster_acknowledged: false,
        }
      : command === "gpu_telemetry_capabilities"
        ? {
            allowed: true,
            message: null,
            available_families: ["utilization"],
            identity_labels: ["UUID", "pod_uid", "container", "node"],
          }
        : command === "gpu_telemetry_history"
          ? {
              ...history,
              series: [
                {
                  ...history.series[0],
                  labels: {
                    UUID: "GPU-gone",
                    namespace: "team",
                    pod: "train",
                    pod_uid: "uid",
                    container: "worker",
                    node: "node",
                    lumen_cluster_provenance: "verified",
                  },
                },
              ],
            }
          : command === "gpu_inventory_snapshot"
            ? { pods: source([]), nodes: source([]), namespace: "team" }
            : command === "device_resources_snapshot"
              ? snapshot()
              : null,
  );
  mount();
  await screen.findByText("GPU-gone");
  await screen.findByText(/Current DRA mapping captured/);
  await userEvent.selectOptions(
    screen.getByLabelText("Current workload association"),
    "uid",
  );
  expect(screen.getByText(/· verified ·/)).toBeInTheDocument();
  ambiguous = true;
  await userEvent.click(
    screen.getByRole("button", { name: "Refresh history" }),
  );
  expect(await screen.findByText(/· unverified ·/)).toBeInTheDocument();
  expect(screen.getByText("GPU-gone")).toBeInTheDocument();
});
it("discards the old configured Service response after saving another source", async () => {
  let saved = { ...config };
  let resolve!: (v: typeof history) => void;
  vi.mocked(invoke).mockImplementation(async (command, args) =>
    command === "gpu_telemetry_config_get"
      ? saved
      : command === "gpu_telemetry_config_set"
        ? ((saved = (args as { config: typeof config }).config), null)
        : command === "gpu_telemetry_capabilities"
          ? {
              allowed: true,
              message: null,
              available_families: ["utilization"],
              identity_labels: [],
            }
          : command === "gpu_telemetry_history"
            ? saved.service === "prom"
              ? new Promise((r) => {
                  resolve = r;
                })
              : {
                  ...history,
                  series: [
                    {
                      ...history.series[0],
                      labels: { UUID: "GPU-new-service" },
                    },
                  ],
                }
            : Promise.reject(new Error("no mapping")),
  );
  mount();
  await screen.findByText("Loading historical readings…");
  const field = screen.getByLabelText("Service name");
  await userEvent.clear(field);
  await userEvent.type(field, "another");
  await userEvent.click(screen.getByRole("button", { name: "Save source" }));
  await screen.findByText("GPU-new-service");
  await act(async () => resolve(history));
  expect(screen.queryByText("GPU-gone")).not.toBeInTheDocument();
  expect(screen.getByText("GPU-new-service")).toBeInTheDocument();
});
it("rechecks capabilities after saving the same source to recover from denial", async () => {
  let allowed = false;
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "gpu_telemetry_config_get"
      ? config
      : command === "gpu_telemetry_config_set"
        ? ((allowed = true), null)
        : command === "gpu_telemetry_capabilities"
          ? {
              allowed,
              message: null,
              available_families: allowed ? ["utilization"] : [],
              identity_labels: [],
            }
          : command === "gpu_telemetry_history"
            ? history
            : Promise.reject(new Error("no mapping")),
  );
  mount();
  await screen.findByText(/Service proxy access unavailable/);
  await userEvent.click(screen.getByRole("button", { name: "Save source" }));
  expect(await screen.findByText("GPU-gone")).toBeInTheDocument();
});

it("splits absent evaluations and marks trailing disappearance unavailable", async () => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args) => command === "gpu_telemetry_history" ? {
    ...history, interval: {start: 0, end: 600, step: 100}, series: [{...history.series[0], points: [[0, 10], [400, 50]]}]
  } : original(command, args));
  mount();
  await screen.findByText("GPU-gone");
  expect(screen.getByText(/5 gaps/)).toBeInTheDocument();
  expect(screen.getByText(/Latest: Unavailable/)).toBeInTheDocument();
  expect(document.querySelectorAll("polyline")).toHaveLength(2);
});

it.each([
  {points: [[0, 10], [600, 50]], gaps: 5, latest: "50", segments: 2},
  {points: [[200, 10], [300, 50], [400, 50], [500, 50], [600, 50]], gaps: 2, latest: "50", segments: 1},
])("represents absent timestamps without changing original observations: %j", async ({points, gaps, latest, segments}) => {
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (command, args) => command === "gpu_telemetry_history" ? {
    ...history, interval: {start: 0, end: 600, step: 100}, series: [{...history.series[0], points}]
  } : original(command, args));
  mount(); await screen.findByText("GPU-gone");
  expect(screen.getByText(new RegExp(`${gaps} gaps`))).toBeInTheDocument();
  expect(screen.getByText(new RegExp(`Latest: ${latest}`))).toBeInTheDocument();
  expect(document.querySelectorAll("polyline")).toHaveLength(segments);
});
