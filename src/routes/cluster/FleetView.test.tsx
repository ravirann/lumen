import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { FleetView } from "./FleetView";
import {
  k8s,
  type ContextInfo,
  type DeletedContextSummary,
  type FleetCard,
  type FleetHealth,
} from "@/lib/k8s";
import { useClusterStore } from "@/state/cluster";
import { diagnoseConnection } from "@/lib/connectionDiagnostics";

vi.mock("@/lib/k8s", () => ({
  k8s: {
    listContexts: vi.fn(),
    setContext: vi.fn(),
    probeFleetContext: vi.fn(),
    disconnectContext: vi.fn(),
    deleteContext: vi.fn(),
    listDeletedContexts: vi.fn(),
    restoreDeletedContext: vi.fn(),
  },
}));

vi.mock("@/lib/connectionDiagnostics", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/connectionDiagnostics")>()),
  diagnoseConnection: vi.fn(),
}));

function fleetCard(
  overrides: Omit<Partial<FleetCard>, "context" | "health"> & {
    name: string;
    context?: Partial<ContextInfo>;
    health?: Partial<FleetHealth>;
  },
): FleetCard {
  const { context, health, name, ...rest } = overrides;
  return {
    context: {
      name,
      cluster: `${name}-cluster`,
      user: `${name}-user`,
      namespace: "default",
      is_current: false,
      is_prod: false,
      ...context,
    },
    reachable: true,
    error: null,
    server_version: "v1.29.0",
    node_count: 3,
    node_ready: 3,
    namespace_count: 12,
    workload_count: 40,
    health: {
      pods_total: 30,
      pods_ready: 30,
      pods_pending: 0,
      pods_failed: 0,
      ...health,
    },
    cpu_percent: 35,
    mem_percent: 40,
    fetched_at_ms: 1_700_000_000_000,
    ...rest,
  };
}

function LocationProbe() {
  const location = useLocation();
  return <div data-testid="location">{location.pathname}</div>;
}

function ClusterRouteProbe() {
  const nav = useNavigate();
  return (
    <>
      <LocationProbe />
      <button type="button" onClick={() => nav("/cluster")}>
        back to fleet
      </button>
    </>
  );
}

function renderFleet(cards: FleetCard[]) {
  let contexts = cards.map((card) => card.context);
  let trash: DeletedContextSummary[] = [];
  vi.mocked(k8s.listContexts).mockImplementation(async () => contexts);
  vi.mocked(k8s.listDeletedContexts).mockImplementation(async () => trash);
  vi.mocked(k8s.setContext).mockImplementation(async (name: string) => {
    const context = contexts.find((context) => context.name === name);
    if (!context) throw new Error(`missing context ${name}`);
    return context;
  });
  vi.mocked(k8s.probeFleetContext).mockImplementation(async (name: string) => {
    const card = cards.find((c) => c.context.name === name);
    if (!card) throw new Error(`missing card ${name}`);
    return card;
  });
  vi.mocked(k8s.disconnectContext).mockResolvedValue();
  vi.mocked(k8s.deleteContext).mockImplementation(async (name: string) => {
    const context = contexts.find((context) => context.name === name);
    contexts = contexts.filter((context) => context.name !== name);
    if (context) {
      trash = [
        ...trash.filter((entry) => entry.name !== name),
        {
          name,
          cluster: context.cluster,
          user: context.user,
          namespace: context.namespace,
          is_prod: context.is_prod,
          deleted_at_ms: 1_700_000_000_000,
          expires_at_ms: 1_707_776_000_000,
          days_remaining: 90,
          has_conflict: false,
        },
      ];
    }
  });
  vi.mocked(k8s.restoreDeletedContext).mockImplementation(async (name: string) => {
    const card = cards.find((card) => card.context.name === name);
    if (card) contexts = [...contexts, card.context];
    trash = trash.filter((entry) => entry.name !== name);
  });
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={["/cluster"]}>
        <Routes>
          <Route
            path="/cluster"
            element={
              <>
                <FleetView />
                <LocationProbe />
              </>
            }
          />
          <Route path="/cluster/:ctx/*" element={<ClusterRouteProbe />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("FleetView", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    window.sessionStorage.clear();
    window.localStorage.clear();
    useClusterStore.setState({
      contextName: null,
      namespace: null,
      lastNamespaceByContext: {},
    });
    vi.mocked(diagnoseConnection).mockResolvedValue({
      context: "unreachable",
      config_path: "/tmp/config",
      status: "ready_to_retry",
      credential_executable: null,
      credential_executable_available: null,
      message: "Configuration inspection passed. Retry to test actual cluster access.",
      single_source_only: true,
    });
  });

  it("keeps a cluster with incomplete inventory accessible without reporting healthy zeros", async () => {
    renderFleet([fleetCard({ name: "slow", error: "Inventory incomplete: pods. Open the cluster or retry to load missing data." })]);
    await userEvent.click(await screen.findByRole("button", { name: /^connect slow$/i }));
    expect(await screen.findByText(/Inventory incomplete: pods/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "slow" })).toBeEnabled();
    expect(screen.queryByText("Healthy", { exact: true })).not.toBeInTheDocument();
    expect(screen.queryByText("0 unhealthy", { exact: true })).not.toBeInTheDocument();
    expect(screen.queryByText("3/3 ready", { exact: true })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "slow" }));
    expect(await screen.findByTestId("location")).toHaveTextContent("/cluster/slow");
  });

  it("offers diagnostics on an unreachable card and safely retries", async () => {
    renderFleet([fleetCard({ name: "unreachable", reachable: false, error: "dial tcp refused" })]);
    await userEvent.click(await screen.findByRole("button", { name: /connect unreachable/i }));
    await userEvent.click(await screen.findByRole("button", { name: /diagnose unreachable/i }));
    expect(await screen.findByRole("dialog", { name: /connection diagnostics/i })).toBeInTheDocument();
    expect(diagnoseConnection).toHaveBeenCalledWith("unreachable");
    expect(
      within(screen.getByRole("dialog", { name: /connection diagnostics/i })).getByText(
        "Cluster network unavailable",
      ),
    ).toBeInTheDocument();
    vi.mocked(k8s.probeFleetContext).mockResolvedValue(fleetCard({ name: "unreachable" }));
    await userEvent.click(screen.getByRole("button", { name: /retry connection/i }));
    expect(k8s.probeFleetContext).toHaveBeenCalledWith("unreachable");
    await waitFor(() =>
      expect(
        within(screen.getByRole("dialog", { name: /connection diagnostics/i })).queryByText(
          "Cluster network unavailable",
        ),
      ).not.toBeInTheDocument(),
    );
  });

  it("provides actionable diagnostics when no contexts exist", async () => {
    renderFleet([]);
    await userEvent.click(await screen.findByRole("button", { name: /diagnose kubeconfig/i }));
    expect(await screen.findByRole("dialog", { name: /connection diagnostics/i })).toBeInTheDocument();
    expect(diagnoseConnection).toHaveBeenCalledWith(null);
  });

  it("captures a rejected retry as the latest safe diagnostic error", async () => {
    renderFleet([fleetCard({ name: "unstable", reachable: false, error: "old timeout" })]);
    await userEvent.click(await screen.findByRole("button", { name: /connect unstable/i }));
    vi.mocked(k8s.probeFleetContext).mockRejectedValueOnce(
      new Error("Forbidden token=never-render-this"),
    );
    await userEvent.click(await screen.findByRole("button", { name: /diagnose unstable/i }));
    await screen.findByRole("dialog", { name: /connection diagnostics/i });
    await userEvent.click(screen.getByRole("button", { name: /retry connection/i }));
    expect(
      await within(screen.getByRole("dialog", { name: /connection diagnostics/i })).findByText(
        "Permission denied",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/never-render-this/)).not.toBeInTheDocument();
  });

  it("orders cluster cards by operator risk", async () => {
    renderFleet([
      fleetCard({ name: "healthy-dev" }),
      fleetCard({
        name: "failed-prod",
        context: { is_prod: true },
        health: { pods_failed: 2 },
      }),
      fleetCard({ name: "pressure", cpu_percent: 93 }),
      fleetCard({ name: "unreachable", reachable: false, error: "network down" }),
    ]);

    await userEvent.click(await screen.findByRole("button", { name: /connect unreachable/i }));
    await userEvent.click(await screen.findByRole("button", { name: /connect failed-prod/i }));
    await userEvent.click(await screen.findByRole("button", { name: /connect pressure/i }));

    const grid = await screen.findByTestId("fleet-card-grid");
    const cards = within(grid).getAllByTestId("fleet-card");

    expect(cards.map((card) => card.textContent)).toEqual([
      expect.stringContaining("unreachable"),
      expect.stringContaining("failed-prod"),
      expect.stringContaining("pressure"),
      expect.stringContaining("healthy-dev"),
    ]);
  });

  it("opens a cluster directly into workload triage", async () => {
    renderFleet([fleetCard({ name: "aks-stage" })]);

    await userEvent.click(await screen.findByRole("button", { name: /^aks-stage$/i }));

    expect(screen.getByTestId("location")).toHaveTextContent(
      "/cluster/aks-stage/workloads",
    );
  });

  it("opens connected clusters from the triage CTA", async () => {
    renderFleet([fleetCard({ name: "aks-prod" })]);

    await userEvent.click(await screen.findByRole("button", { name: /connect aks-prod/i }));
    await userEvent.click(await screen.findByRole("button", { name: /triage aks-prod/i }));

    expect(screen.getByTestId("location")).toHaveTextContent(
      "/cluster/aks-prod/workloads",
    );
  });

  it("separates workflow, connection, and danger actions on connected cards", async () => {
    renderFleet([fleetCard({ name: "aks-prod" })]);

    await userEvent.click(await screen.findByRole("button", { name: /connect aks-prod/i }));
    const card = (await screen.findByRole("button", { name: /triage aks-prod/i })).closest(
      "[data-testid='fleet-card']",
    );
    expect(card).not.toBeNull();

    const workflow = within(card as HTMLElement).getByTestId("fleet-card-workflow-actions");
    const connection = within(card as HTMLElement).getByTestId("fleet-card-connection-actions");
    const danger = within(card as HTMLElement).getByTestId("fleet-card-danger-actions");
    expect(within(workflow).getByRole("button", { name: /triage aks-prod/i })).toBeInTheDocument();
    expect(within(connection).getByRole("button", { name: /disconnect aks-prod/i })).toBeInTheDocument();
    expect(within(danger).getByRole("button", { name: /delete aks-prod/i })).toBeInTheDocument();
    expect(within(danger).queryByRole("button", { name: /triage aks-prod/i })).not.toBeInTheDocument();
  });

  it("preserves connected cluster state after returning to fleet", async () => {
    renderFleet([fleetCard({ name: "persisted-prod", context: { is_prod: true } })]);

    await userEvent.click(await screen.findByRole("button", { name: /connect persisted-prod/i }));
    expect(await screen.findByRole("button", { name: /disconnect persisted-prod/i })).toBeInTheDocument();
    expect(k8s.setContext).toHaveBeenCalledWith("persisted-prod");
    expect(useClusterStore.getState().contextName).toBe("persisted-prod");
    expect(window.localStorage.getItem("lumen-cluster")).toContain("persisted-prod");

    await userEvent.click(screen.getByRole("button", { name: /triage persisted-prod/i }));
    expect(screen.getByTestId("location")).toHaveTextContent(
      "/cluster/persisted-prod/workloads",
    );

    await userEvent.click(screen.getByRole("button", { name: /back to fleet/i }));
    expect(await screen.findByRole("button", { name: /disconnect persisted-prod/i })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^connect persisted-prod$/i }),
    ).not.toBeInTheDocument();
  });

  it("keeps connect separate from opening the discovered cluster", async () => {
    renderFleet([fleetCard({ name: "prod-main" })]);

    expect(await screen.findByRole("button", { name: /^prod-main$/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /connect prod-main/i })).toBeInTheDocument();
  });

  it("deletes a cluster from local kubeconfig so rescan does not restore it", async () => {
    const first = fleetCard({ name: "hidden-dev" });
    renderFleet([first]);

    expect(await screen.findByText("hidden-dev")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /delete hidden-dev/i }));
    const dialog = screen.getByRole("dialog", { name: /delete cluster from fleet/i });
    expect(screen.getByText(/moves the context to trash for 90 days/i)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("button", { name: /^delete$/i }));
    expect(k8s.deleteContext).toHaveBeenCalledWith("hidden-dev");
    expect(screen.queryByRole("button", { name: /^hidden-dev$/i })).not.toBeInTheDocument();
    const trashToggle = await screen.findByRole("button", { name: /trash/i });
    expect(trashToggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("button", { name: /restore hidden-dev/i })).not.toBeInTheDocument();
    await userEvent.click(trashToggle);
    expect(screen.getByRole("button", { name: /restore hidden-dev/i })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /rescan/i }));
    expect(screen.queryByRole("button", { name: /^hidden-dev$/i })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /restore hidden-dev/i })).toBeInTheDocument();
  });

  it("restores a deleted cluster from trash", async () => {
    renderFleet([fleetCard({ name: "restore-dev" })]);

    await userEvent.click(await screen.findByRole("button", { name: /delete restore-dev/i }));
    await userEvent.click(
      within(screen.getByRole("dialog", { name: /delete cluster from fleet/i })).getByRole(
        "button",
        { name: /^delete$/i },
      ),
    );
    await userEvent.click(await screen.findByRole("button", { name: /trash/i }));
    await userEvent.click(await screen.findByRole("button", { name: /restore restore-dev/i }));

    expect(k8s.restoreDeletedContext).toHaveBeenCalledWith("restore-dev", false);
    expect(await screen.findByText("restore-dev")).toBeInTheDocument();
    expect(screen.queryByText("trash")).not.toBeInTheDocument();
  });

  it("lets users add multiple persistent labels to a cluster", async () => {
    renderFleet([fleetCard({ name: "labelled-dev" })]);

    const input = await screen.findByRole("textbox", { name: /add label to labelled-dev/i });
    await userEvent.type(input, "stage{enter}");
    await userEvent.type(input, "payments{enter}");

    expect(screen.getByText("stage")).toBeInTheDocument();
    expect(screen.getByText("payments")).toBeInTheDocument();
    expect(window.localStorage.getItem("lumen:fleet:cluster-labels")).toContain("payments");
  });
});
