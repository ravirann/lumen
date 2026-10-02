export type GpuTelemetryConfig = {
  namespace: string;
  service: string;
  port: string;
  cluster_label: string | null;
  cluster_value: string | null;
  single_cluster_acknowledged: boolean;
};
export type GpuTelemetryCapabilities = {
  allowed: boolean;
  message: string | null;
  available_families: GpuMetricFamily[];
  identity_labels: string[];
};
export type GpuMetricFamily =
  | "utilization"
  | "framebuffer_used"
  | "framebuffer_total"
  | "sm_active"
  | "tensor_active"
  | "xid_errors";
export type GpuSeries = {
  family: GpuMetricFamily;
  labels: Record<string, string>;
  points: [number, number | null][];
};
export type GpuHistory = {
  captured_at: string;
  complete: boolean;
  warnings: string[];
  series: GpuSeries[];
};
