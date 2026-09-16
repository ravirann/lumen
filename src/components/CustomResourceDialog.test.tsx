import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CustomResourceDialog } from "./CustomResourceDialog";
import {
  customResources,
  type CrdDetails,
  type CustomResource,
} from "@/lib/customResources";
import { useUiSettings } from "@/state/uiSettings";

const policy = vi.hoisted(() => ({ locked: false }));
vi.mock("@/lib/customResources", async (original) => ({
  ...(await original<typeof import("@/lib/customResources")>()),
  customResources: {
    get: vi.fn(),
    access: vi.fn(),
    write: vi.fn(),
    delete: vi.fn(),
  },
}));
vi.mock("@/hooks/useMutationCapability", () => ({
  useMutationCapability: () => {
    const readOnly = useUiSettings((s) => s.readOnly);
    return {
      globalReadOnly: readOnly,
      canMutate: !readOnly && !policy.locked,
      reason: readOnly
        ? "Global read-only mode is enabled"
        : policy.locked
          ? "Protected context is locked"
          : "Changes allowed",
    };
  },
}));
const crd: CrdDetails = {
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
      columns: [],
      schema: { type: "object" },
    },
  ],
};
const yaml =
  'apiVersion: example.io/v1\nkind: Widget\nmetadata:\n  name: sample\n  namespace: team\n  uid: original-uid\n  resourceVersion: "7"\nspec:\n  replicas: 1\n';
const resource: CustomResource = {
  name: "sample",
  namespace: "team",
  uid: "original-uid",
  resource_version: "7",
  generation: 2,
  conditions: [],
  printer_cells: [],
  age_seconds: 50,
  status_hint: null,
};
const close = vi.fn();
function mount(create = false, scope = "Namespaced") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <CustomResourceDialog
        context="dev"
        crd={{ ...crd, scope }}
        version="v1"
        namespace="team"
        resource={create ? undefined : resource}
        onClose={close}
      />
    </QueryClientProvider>,
  );
  return { ...view, client };
}
beforeEach(() => {
  vi.clearAllMocks();
  policy.locked = false;
  useUiSettings.setState({ readOnly: false });
  vi.mocked(customResources.get).mockResolvedValue({
    yaml,
    uid: "original-uid",
    resource_version: "7",
    generation: 2,
    conditions: [
      {
        type: "Ready",
        status: "False",
        reason: "Reconciling",
        message: "Waiting for deployment",
        observed_generation: 1,
        last_transition_time: null,
      },
    ],
  });
  vi.mocked(customResources.access).mockResolvedValue({
    allowed: true,
    denied: false,
    reason: null,
    evaluation_error: null,
  });
  vi.mocked(customResources.write).mockImplementation(
    async (_target, manifest, _create, dry_run) => ({
      yaml: manifest,
      dry_run,
    }),
  );
  vi.mocked(customResources.delete).mockResolvedValue(undefined);
});
describe("custom resource operations", () => {
  it("cancels deletion approval when the loaded identity changes", async () => {
    const user = userEvent.setup();
    const { client } = mount();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Delete" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await user.type(screen.getByLabelText("Confirmation name"), "sample");
    expect(screen.getByRole("button", { name: "Reload" })).toBeDisabled();
    act(() => {
      client.setQueriesData(
        { queryKey: ["k8s", "custom-resource"] },
        (old: any) => ({
          ...old,
          uid: "replacement-uid",
          resource_version: "8",
        }),
      );
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Confirm deletion" }),
      ).not.toBeInTheDocument(),
    );
    expect(customResources.delete).not.toHaveBeenCalled();
  });
  it("shows conditions and freshness; applies only an exactly validated draft with same-GET identity", async () => {
    const user = userEvent.setup();
    mount();
    expect(
      await screen.findByText("Waiting for deployment", { exact: false }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Stale condition/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Edit YAML" }));
    expect(
      screen.getByRole("button", { name: "Apply changes" }),
    ).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Custom resource manifest"), {
      target: { value: yaml.replace("replicas: 1", "replicas: 2") },
    });
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Apply changes" }),
      ).toBeEnabled(),
    );
    fireEvent.change(screen.getByLabelText("Custom resource manifest"), {
      target: { value: yaml.replace("replicas: 1", "replicas: 3") },
    });
    expect(
      screen.getByRole("button", { name: "Apply changes" }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Apply changes" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Apply changes" }));
    await user.type(screen.getByLabelText("Confirmation name"), "sample");
    await user.click(screen.getByRole("button", { name: "Confirm update" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
    expect(customResources.write).toHaveBeenLastCalledWith(
      expect.objectContaining({
        uid: "original-uid",
        resource_version: "7",
        namespace: "team",
        name: "sample",
      }),
      yaml.replace("replicas: 1", "replicas: 3"),
      false,
      false,
      "dev",
    );
  });
  it("creates with explicit namespace and requires server validation plus confirmation", async () => {
    const user = userEvent.setup();
    mount(true);
    await user.type(screen.getByLabelText("Resource name"), "new-widget");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Edit manifest" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Edit manifest" }));
    expect(
      screen.getByRole("button", { name: "Create resource" }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Create resource" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Create resource" }));
    await user.type(screen.getByLabelText("Confirmation name"), "new-widget");
    await user.click(screen.getByRole("button", { name: "Confirm creation" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
    expect(customResources.write).toHaveBeenLastCalledWith(
      expect.objectContaining({ name: "new-widget", namespace: "team" }),
      expect.stringContaining('namespace: "team"'),
      true,
      false,
      "dev",
    );
  });
  it("omits namespace for cluster-scoped creation", async () => {
    const user = userEvent.setup();
    mount(true, "Cluster");
    expect(
      screen.queryByLabelText("Resource namespace"),
    ).not.toBeInTheDocument();
    await user.type(screen.getByLabelText("Resource name"), "global-widget");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Edit manifest" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Edit manifest" }));
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    await waitFor(() => expect(customResources.write).toHaveBeenCalled());
    expect(customResources.write).toHaveBeenCalledWith(
      expect.objectContaining({ namespace: null }),
      expect.not.stringContaining("namespace:"),
      true,
      true,
      "dev",
    );
  });
  it("requires typed confirmation and captured UID for deletion", async () => {
    const user = userEvent.setup();
    mount();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Delete" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(
      screen.getByRole("button", { name: "Confirm deletion" }),
    ).toBeDisabled();
    await user.type(screen.getByLabelText("Confirmation name"), "sample");
    await user.click(screen.getByRole("button", { name: "Confirm deletion" }));
    await waitFor(() =>
      expect(customResources.delete).toHaveBeenCalledWith(
        expect.objectContaining({ uid: "original-uid", resource_version: "7" }),
        "dev",
      ),
    );
  });
  it("blocks denied mutation permissions", async () => {
    vi.mocked(customResources.access).mockResolvedValue({
      allowed: false,
      denied: true,
      reason: "Forbidden",
      evaluation_error: null,
    });
    mount();
    await screen.findByText(/Cluster permissions do not allow editing/);
    expect(screen.getByRole("button", { name: "Edit YAML" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
  });
  it("allows validation in a locked context but prevents apply and delete", async () => {
    policy.locked = true;
    mount();
    const user = userEvent.setup();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Edit YAML" })).toBeEnabled(),
    );
    expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Edit YAML" }));
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    await screen.findByText(/Server validation passed/);
    expect(
      screen.getByRole("button", { name: "Apply changes" }),
    ).toBeDisabled();
  });
  it("invalidates an in-flight validation when global read-only is enabled", async () => {
    let resolve!: (value: { yaml: string; dry_run: boolean }) => void;
    vi.mocked(customResources.write).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const user = userEvent.setup();
    mount();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Edit YAML" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Edit YAML" }));
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    act(() => useUiSettings.setState({ readOnly: true }));
    await act(async () => resolve({ yaml, dry_run: true }));
    expect(
      screen.queryByText(/Server validation passed/),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Apply changes" }),
    ).toBeDisabled();
    expect(screen.getByLabelText("Custom resource manifest")).toBeDisabled();
  });
  it("does not allow failed validation to authorize a write", async () => {
    vi.mocked(customResources.write).mockRejectedValue({
      kind: "K8s",
      message: "Required field missing",
    });
    const user = userEvent.setup();
    mount();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Edit YAML" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Edit YAML" }));
    await user.click(
      screen.getByRole("button", { name: "Validate on server" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Required field missing",
    );
    expect(
      screen.getByRole("button", { name: "Apply changes" }),
    ).toBeDisabled();
  });
});
