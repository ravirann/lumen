import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { GpuTelemetrySetup } from "./GpuTelemetrySetup";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const mount = () =>
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <GpuTelemetrySetup context="demo" onSaved={vi.fn()} />
    </QueryClientProvider>,
  );
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(invoke).mockResolvedValue(null);
});
it("requires explicit cluster provenance and explains proxy data access", async () => {
  mount();
  await screen.findByText("Configure existing Prometheus Service");
  expect(
    screen.getByText(/Namespace filters scope queries/),
  ).toBeInTheDocument();
  for (const [label, value] of [
    ["Service namespace", "metrics"],
    ["Service name", "prometheus"],
    ["Service port", "9090"],
  ])
    await userEvent.type(screen.getByLabelText(label), value);
  expect(screen.getByRole("button", { name: "Save source" })).toBeDisabled();
  await userEvent.click(screen.getByLabelText(/This source contains only/));
  await userEvent.click(screen.getByRole("button", { name: "Save source" }));
  expect(invoke).toHaveBeenCalledWith("gpu_telemetry_config_set", {
    context: "demo",
    config: {
      namespace: "metrics",
      service: "prometheus",
      port: "9090",
      cluster_label: null,
      cluster_value: null,
      single_cluster_acknowledged: true,
    },
  });
});
it("keeps persistence failure secret-safe", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "gpu_telemetry_config_set")
      throw new Error("Bearer SECRET");
    return null;
  });
  mount();
  await screen.findByText("Configure existing Prometheus Service");
  for (const [label, value] of [
    ["Service namespace", "metrics"],
    ["Service name", "prometheus"],
    ["Service port", "9090"],
    ["Cluster label", "cluster"],
    ["Cluster value", "demo"],
  ])
    await userEvent.type(screen.getByLabelText(label), value);
  await userEvent.click(screen.getByRole("button", { name: "Save source" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
  expect(screen.queryByText(/SECRET/)).not.toBeInTheDocument();
});
it("tests saved source and removes it without displaying backend errors", async () => {
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "gpu_telemetry_config_get"
      ? {
          namespace: "m",
          service: "p",
          port: "9090",
          cluster_label: "cluster",
          cluster_value: "demo",
          single_cluster_acknowledged: false,
        }
      : command === "gpu_telemetry_capabilities"
        ? {
            allowed: false,
            message: "SECRET",
            available_families: [],
            identity_labels: [],
          }
        : null,
  );
  mount();
  await screen.findByText("Configure existing Prometheus Service");
  await userEvent.click(
    screen.getByRole("button", { name: "Test saved source" }),
  );
  expect(
    await screen.findByText(/Service proxy access unavailable/),
  ).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Remove source" }));
  expect(invoke).toHaveBeenCalledWith("gpu_telemetry_config_set", {
    context: "demo",
    config: null,
  });
  expect(screen.queryByText(/SECRET/)).not.toBeInTheDocument();
});
