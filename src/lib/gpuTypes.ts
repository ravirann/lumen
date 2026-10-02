export type SourceState = "available" | "unsupported" | "forbidden" | "error";
export type Source<T> = { state: SourceState; complete: boolean; captured_at: string; items: T[]; message: string | null };
export type ResourceRef = { api_version: string; kind: string; namespace: string | null; name: string; uid: string };
export type Explanation = { stage: "admission" | "scheduling" | "startup"; confidence: "observed" | "inferred" | "unknown"; message: string; sources: ResourceRef[]; captured_at: string; transition_time: string | null; observed_generation: number | null };
export type SchedulingSnapshot = { pod: ResourceRef; sources: Record<string, Source<Record<string, unknown>>>; explanations: Explanation[] };
export type GpuPod = { resource: ResourceRef; phase: string; node_name: string | null; requests: Record<string, number | null>; owners: ResourceRef[] };
export type GpuNode = { resource: ResourceRef; allocatable: Record<string, number | null>; gpu_labels: Record<string, string> };
