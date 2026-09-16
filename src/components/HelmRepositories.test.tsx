import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { helmRepositories } from "@/lib/helmRepositories";
import { HelmRepositories } from "./HelmRepositories";

vi.mock("@/lib/helmRepositories", () => ({
  helmRepositories: {
    list: vi.fn(),
    add: vi.fn(),
    remove: vi.fn(),
    update: vi.fn(),
  },
}));
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  render(
    <QueryClientProvider client={client}>
      <HelmRepositories />
    </QueryClientProvider>,
  );
  fireEvent.click(screen.getByText("Manage Helm repositories"));
  return invalidate;
}
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(helmRepositories.list).mockResolvedValue([
    { name: "stable", url: "https://charts.example.org" },
  ]);
  vi.mocked(helmRepositories.add).mockResolvedValue(undefined);
  vi.mocked(helmRepositories.remove).mockResolvedValue(undefined);
  vi.mocked(helmRepositories.update).mockResolvedValue(undefined);
});
describe("local Helm repositories", () => {
  it("adds a repository and refreshes chart search", async () => {
    const invalidate = setup();
    await screen.findByText("stable");
    expect(
      screen.getByText(/all clusters on this machine/),
    ).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Repository name"), {
      target: { value: "new-repo" },
    });
    fireEvent.change(screen.getByLabelText("Repository URL"), {
      target: { value: "https://new.example.org" },
    });
    fireEvent.click(screen.getByRole("button", { name: "add repository" }));
    await screen.findByText("Repository added.");
    expect(helmRepositories.add).toHaveBeenCalledWith(
      "new-repo",
      "https://new.example.org",
    );
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["helm-search"] });
  });
  it("requires confirmation before removing a local repository", async () => {
    setup();
    fireEvent.click(
      await screen.findByRole("button", { name: "Remove repository stable" }),
    );
    expect(helmRepositories.remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "confirm removal" }));
    await waitFor(() =>
      expect(helmRepositories.remove).toHaveBeenCalledWith("stable"),
    );
    await screen.findByText(
      /Repository removed. Installed releases are unchanged/,
    );
  });
  it("reports update failure without claiming success", async () => {
    vi.mocked(helmRepositories.update).mockRejectedValue(
      new Error("Repository update failed"),
    );
    setup();
    await screen.findByText("stable");
    fireEvent.click(screen.getByRole("button", { name: "refresh indexes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Repository update failed",
    );
    expect(
      screen.queryByText("Repository indexes refreshed."),
    ).not.toBeInTheDocument();
  });
});
