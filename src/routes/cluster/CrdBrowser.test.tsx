import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useNavigate } from "react-router-dom";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrdBrowser } from "./CrdBrowser";
import { customResources, type CrdDetails } from "@/lib/customResources";
import { k8s } from "@/lib/k8s";
import { useUiSettings } from "@/state/uiSettings";
globalThis.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
};
Element.prototype.scrollIntoView = vi.fn();
vi.mock("@/lib/k8s", () => ({
  k8s: { listCrds: vi.fn(), listNamespaces: vi.fn(), listContexts: vi.fn() },
}));
vi.mock("@/lib/customResources", async (original) => ({
  ...(await original<typeof import("@/lib/customResources")>()),
  customResources: { details: vi.fn(), list: vi.fn() },
}));
vi.mock("@/hooks/useMutationCapability", () => ({
  useMutationCapability: () => ({
    globalReadOnly: false,
    canMutate: true,
    reason: "Changes allowed",
  }),
}));
vi.mock("@/components/CustomResourceDialog", () => ({
  CustomResourceDialog: ({ resource, version }: any) => (
    <div role="dialog">
      Inspecting {resource?.name} {version}
    </div>
  ),
}));
const column = {
  name: "Replicas",
  type: "integer",
  description: "Desired replicas",
  json_path: ".spec.replicas",
  priority: 0,
};
const details: CrdDetails = {
  name: "widgets.example.io",
  kind: "Widget",
  plural: "widgets",
  group: "example.io",
  scope: "Namespaced",
  versions: [
    {
      name: "v1",
      served: true,
      storage: true,
      columns: [column],
      schema: { type: "object" },
    },
    { name: "v2", served: true, storage: false, columns: [], schema: null },
    { name: "v0", served: false, storage: false, columns: [], schema: null },
  ],
};
function Navigation() {
  const navigate = useNavigate();
  return (
    <>
      <button onClick={() => navigate("/cluster/other/crds?ns=other")}>
        Switch context
      </button>
      <CrdBrowser />
    </>
  );
}
function mount() {
  return render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <MemoryRouter initialEntries={["/cluster/dev/crds?ns=team"]}>
        <Routes>
          <Route path="/cluster/:ctx/crds" element={<Navigation />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  useUiSettings.setState({ selectedNamespaces: {} });
  vi.mocked(k8s.listCrds).mockResolvedValue([
    {
      name: details.name,
      group: details.group,
      kind: details.kind,
      plural: details.plural,
      short_names: [],
      scope: details.scope,
      versions: ["v1", "v2", "v0"],
      preferred_version: "v1",
      age_seconds: 50,
    },
  ]);
  vi.mocked(k8s.listNamespaces).mockResolvedValue(["team", "other"]);
  vi.mocked(k8s.listContexts).mockResolvedValue([]);
  vi.mocked(customResources.details).mockResolvedValue(details);
  vi.mocked(customResources.list).mockResolvedValue({
    columns: [column],
    items: [
      {
        name: "sample",
        namespace: "team",
        uid: "uid",
        resource_version: "1",
        generation: 1,
        conditions: [],
        status_hint: "Ready=True",
        age_seconds: 60,
        printer_cells: [{ name: "Replicas", value: 3, supported: true }],
      },
    ],
  });
});
describe("CRD explorer", () => {
  it("shows printer columns and queries served versions within selected namespace", async () => {
    mount();
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Select Widget" }),
    );
    expect(
      await screen.findByRole("columnheader", { name: "Replicas" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "3" })).toBeInTheDocument();
    expect(customResources.list).toHaveBeenCalledWith(
      details.name,
      "v1",
      "team",
      "dev",
    );
    expect(
      screen.queryByRole("option", { name: "v0" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Inspect sample" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await user.selectOptions(
      screen.getByRole("combobox", { name: "CRD version" }),
      "v2",
    );
    await waitFor(() =>
      expect(customResources.list).toHaveBeenCalledWith(
        details.name,
        "v2",
        "team",
        "dev",
      ),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it("changes a namespace supplied in the URL and closes the inspector", async () => {
    mount();
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Select Widget" }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Inspect sample" }),
    );
    await user.click(screen.getByRole("button", { name: "namespace: team" }));
    await user.click(screen.getByRole("option", { name: "other" }));
    await waitFor(() =>
      expect(customResources.list).toHaveBeenCalledWith(
        details.name,
        "v1",
        "other",
        "dev",
      ),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "namespace: other" }),
    ).toBeInTheDocument();
  });
  it("drops selected resources on context switch", async () => {
    mount();
    const user = userEvent.setup();
    await user.click(
      await screen.findByRole("button", { name: "Select Widget" }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Inspect sample" }),
    );
    await user.click(screen.getByRole("button", { name: "Switch context" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Inspect sample" }),
    ).not.toBeInTheDocument();
  });
  it("does not present permission failures as an empty resource list", async () => {
    vi.mocked(customResources.list).mockRejectedValue(new Error("Forbidden"));
    mount();
    await userEvent.click(
      await screen.findByRole("button", { name: "Select Widget" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("Forbidden");
    expect(
      screen.queryByText("No custom resources in this scope."),
    ).not.toBeInTheDocument();
  });
  it("marks unsupported printer paths instead of inventing values", async () => {
    vi.mocked(customResources.list).mockResolvedValue({
      columns: [column],
      items: [
        {
          name: "sample",
          namespace: "team",
          uid: "uid",
          resource_version: "1",
          generation: 1,
          conditions: [],
          status_hint: null,
          age_seconds: 5,
          printer_cells: [{ name: "Replicas", value: null, supported: false }],
        },
      ],
    });
    mount();
    await userEvent.click(
      await screen.findByRole("button", { name: "Select Widget" }),
    );
    expect(await screen.findByText("Unsupported path")).toBeInTheDocument();
  });
});
