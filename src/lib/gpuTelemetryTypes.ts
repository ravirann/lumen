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
};
