import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { YamlDiffView } from "@/components/YamlDiffView";
import { useMutationCapability } from "@/hooks/useMutationCapability";
import { useUiSettings } from "@/state/uiSettings";
import { useChangeHistoryStore } from "@/state/changeHistory";
import { errorMessage as errorText } from "@/lib/errorMessage";
import {
  conditionFreshness,
  customResources,
  resourceTemplate,
  type CrdDetails,
  type CrTarget,
  type CustomResource,
} from "@/lib/customResources";

type Props = {
  context: string;
  crd: CrdDetails;
  version: string;
  namespace: string;
  resource?: CustomResource;
  onClose: () => void;
};

const validName = (name: string) =>
  name.length <= 253 && /^[a-z0-9](?:[-a-z0-9.]*[a-z0-9])?$/.test(name);

/** Target-scoped editor. A server validation belongs only to the exact draft submitted. */
export function CustomResourceDialog({
  context,
  crd,
  version,
  namespace,
  resource,
  onClose,
}: Props) {
  const create = !resource;
  const queryClient = useQueryClient();
  const capability = useMutationCapability(context);
  const [name, setName] = useState(resource?.name ?? "");
  const [targetNamespace, setTargetNamespace] = useState(
    resource?.namespace ?? namespace,
  );
  const [mode, setMode] = useState<"inspect" | "setup" | "edit">(
    create ? "setup" : "inspect",
  );
  const [tab, setTab] = useState<"conditions" | "yaml" | "schema">(
    "conditions",
  );
  const [draft, setDraft] = useState("");
  const [baseline, setBaseline] = useState("");
  const [validated, setValidated] = useState<{
    draft: string;
    yaml: string;
  } | null>(null);
  const [confirmation, setConfirmation] = useState<"write" | "delete" | null>(
    null,
  );
  const [typed, setTyped] = useState("");
  const confirmationSnapshot = useRef<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const operation = useRef(0);
  const inFlight = useRef(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      operation.current++;
    };
  }, []);

  const target: CrTarget = {
    crd_name: crd.name,
    version,
    name,
    namespace: crd.scope === "Namespaced" ? targetNamespace : null,
  };
  const targetValid =
    validName(name) &&
    (target.namespace === null ||
      (targetNamespace.length <= 63 &&
        /^[a-z0-9](?:[-a-z0-9]*[a-z0-9])?$/.test(targetNamespace)));
  const detail = useQuery({
    queryKey: [
      "k8s",
      "custom-resource",
      context,
      crd.name,
      version,
      target.namespace,
      name,
      resource?.uid,
      resource?.resource_version,
    ],
    queryFn: () => customResources.get(target, context),
    enabled: !create,
    refetchOnWindowFocus: false,
    refetchOnMount: "always",
    staleTime: Infinity,
  });
  const guardedTarget: CrTarget = {
    ...target,
    uid: detail.data?.uid,
    resource_version: detail.data?.resource_version,
  };
  const access = useQuery({
    queryKey: [
      "k8s",
      "custom-resource-access",
      context,
      crd.name,
      version,
      target.namespace,
      name,
      create ? "create" : "patch",
    ],
    queryFn: () =>
      customResources.access(target, create ? "create" : "patch", context),
    enabled: targetValid,
    staleTime: 15_000,
  });
  const deleteAccess = useQuery({
    queryKey: [
      "k8s",
      "custom-resource-access",
      context,
      crd.name,
      version,
      target.namespace,
      name,
      "delete",
    ],
    queryFn: () => customResources.access(target, "delete", context),
    enabled: !create,
    staleTime: 15_000,
  });
  const canWrite =
    !capability.globalReadOnly && access.data?.allowed === true && targetValid;
  const stale = mode === "edit" && !create && detail.data?.yaml !== baseline;
  const snapshot = JSON.stringify([context, guardedTarget, draft]);
  const approvalValid = confirmationSnapshot.current === snapshot;
  function requestConfirmation(action: "write" | "delete") {
    confirmationSnapshot.current = snapshot;
    setConfirmation(action);
    setTyped("");
  }
  useEffect(() => {
    if (confirmation && !approvalValid) {
      setConfirmation(null);
      setTyped("");
    }
  }, [confirmation, approvalValid]);

  useEffect(() => {
    if (!capability.canMutate) {
      setConfirmation(null);
      setTyped("");
    }
    if (capability.globalReadOnly) {
      operation.current++;
      setValidated(null);
    }
  }, [capability.canMutate, capability.globalReadOnly]);

  function beginEdit() {
    const yaml = create
      ? resourceTemplate(crd, version, target.namespace, name)
      : detail.data?.yaml;
    if (yaml === undefined || !canWrite) return;
    operation.current++;
    setBaseline(create ? "" : yaml);
    setDraft(yaml);
    setValidated(null);
    setConfirmation(null);
    setError(null);
    setMode("edit");
  }
  function changeDraft(value: string) {
    operation.current++;
    setDraft(value);
    setValidated(null);
    setConfirmation(null);
    setTyped("");
    setError(null);
  }
  async function run(action: "validate" | "write" | "delete") {
    if (inFlight.current || useUiSettings.getState().readOnly) return;
    if (
      action !== "validate" &&
      (!capability.canMutate ||
        typed !== name ||
        !approvalValid ||
        confirmation !== action)
    )
      return;
    if (action !== "delete" && (!canWrite || stale)) return;
    if (action === "write" && validated?.draft !== draft) return;
    if (
      action === "delete" &&
      (!detail.data?.uid || deleteAccess.data?.allowed !== true)
    )
      return;
    const request = ++operation.current;
    const submitted = draft;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    if (action === "validate") setValidated(null);
    try {
      if (action === "delete")
        await customResources.delete(guardedTarget, context);
      else {
        const outcome = await customResources.write(
          guardedTarget,
          submitted,
          create,
          action === "validate",
          context,
        );
        if (action === "validate") {
          if (alive.current && operation.current === request)
            setValidated({ draft: submitted, yaml: outcome.yaml });
          return;
        }
      }
      useChangeHistoryStore.getState().recordEvent({
        action: action === "delete" ? "delete" : "apply",
        target: {
          kind: crd.kind,
          name,
          namespace: target.namespace ?? "",
          context,
        },
        status: "success",
        summary: `${action === "delete" ? "Requested deletion of" : create ? "Created" : "Updated"} ${crd.kind}/${name}`,
      });
      await queryClient.invalidateQueries({
        queryKey: ["k8s", "custom-resources", context, crd.name],
      });
      await queryClient.invalidateQueries({
        queryKey: ["k8s", "custom-resource", context, crd.name],
      });
      toast.success(
        `${action === "delete" ? "Deletion requested for" : create ? "Created" : "Updated"} ${name}`,
      );
      if (alive.current) onClose();
    } catch (e) {
      if (alive.current && operation.current === request) {
        setError(errorText(e));
        setValidated(null);
        setConfirmation(null);
      }
    } finally {
      inFlight.current = false;
      if (alive.current) setBusy(false);
    }
  }
  const conditions = detail.data?.conditions ?? resource?.conditions ?? [];
  const generation = detail.data?.generation ?? resource?.generation ?? null;
  const schema = crd.versions.find((item) => item.name === version)?.schema;
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="flex max-h-[90vh] max-w-5xl flex-col overflow-hidden border-border-default bg-surface text-text-primary">
        <DialogTitle>
          {create ? "Create" : "Inspect"} {crd.kind}
          {name ? ` · ${name}` : ""}
        </DialogTitle>
        <DialogDescription className="break-all text-text-secondary">
          {context} · {target.namespace || "cluster scope"} · {crd.group}/
          {version}
        </DialogDescription>
        {!capability.canMutate && (
          <p className="text-xs text-warning">
            {capability.reason}. Inspection and permitted validation remain
            available.
          </p>
        )}
        {error && (
          <p role="alert" className="whitespace-pre-wrap text-sm text-danger">
            {error}
          </p>
        )}
        {detail.error && (
          <p role="alert" className="text-sm text-danger">
            {errorText(detail.error)}
          </p>
        )}
        {access.error && (
          <p role="alert" className="text-xs text-warning">
            Permission check unavailable: {errorText(access.error)}
          </p>
        )}
        {deleteAccess.error && (
          <p className="text-xs text-warning">
            Delete permission check unavailable: {errorText(deleteAccess.error)}
          </p>
        )}
        {deleteAccess.data && !deleteAccess.data.allowed && (
          <p className="text-xs text-warning">
            Cluster permissions do not allow deletion.{" "}
            {deleteAccess.data.reason}
          </p>
        )}
        {access.data && !access.data.allowed && (
          <p className="text-xs text-warning">
            Cluster permissions do not allow {create ? "creating" : "editing"}{" "}
            this resource. {access.data.reason}
          </p>
        )}
        <div className="min-h-0 flex-1 overflow-auto space-y-4">
          {mode === "setup" && (
            <>
              <label className="block text-sm">
                Resource name
                <Input
                  aria-label="Resource name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className="mt-1"
                />
              </label>
              {crd.scope === "Namespaced" && (
                <label className="block text-sm">
                  Namespace
                  <Input
                    aria-label="Resource namespace"
                    value={targetNamespace}
                    onChange={(e) => setTargetNamespace(e.target.value)}
                    className="mt-1"
                  />
                </label>
              )}
              <p className="text-xs text-text-muted">
                Start with a manifest, then add the fields required by this
                CRD’s schema. The server validates schema, admission, and
                permissions.
              </p>
              <Button disabled={!canWrite} onClick={beginEdit}>
                Edit manifest
              </Button>
            </>
          )}
          {mode === "inspect" && (
            <>
              <div
                className="flex flex-wrap gap-2"
                role="group"
                aria-label="Inspection views"
              >
                {(["conditions", "yaml", "schema"] as const).map((item) => (
                  <Button
                    key={item}
                    size="sm"
                    variant={tab === item ? "secondary" : "ghost"}
                    aria-pressed={tab === item}
                    onClick={() => setTab(item)}
                  >
                    {item === "yaml"
                      ? "YAML"
                      : item === "schema"
                        ? "Schema"
                        : "Conditions"}
                  </Button>
                ))}
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={!detail.data?.yaml}
                  onClick={() => {
                    if (detail.data?.yaml)
                      void navigator.clipboard
                        .writeText(detail.data.yaml)
                        .then(() => toast.success("YAML copied"))
                        .catch((error) => setError(errorText(error)));
                  }}
                >
                  Copy YAML
                </Button>
              </div>
              {detail.isLoading ? (
                <p>Loading resource…</p>
              ) : tab === "yaml" ? (
                <pre className="max-h-[55vh] overflow-auto whitespace-pre font-mono text-xs">
                  {detail.data?.yaml}
                </pre>
              ) : tab === "schema" ? (
                <pre className="max-h-[55vh] overflow-auto whitespace-pre-wrap font-mono text-xs">
                  {schema
                    ? JSON.stringify(schema, null, 2)
                    : "No schema published for this version."}
                </pre>
              ) : (
                <>
                  <p className="text-xs text-text-muted">
                    UID: {detail.data?.uid ?? "unknown"} · generation:{" "}
                    {generation ?? "unknown"} · fetched{" "}
                    {detail.dataUpdatedAt
                      ? new Date(detail.dataUpdatedAt).toLocaleTimeString()
                      : "—"}
                  </p>
                  {conditions.length === 0 ? (
                    <p className="text-sm text-text-secondary">
                      No conditions reported. This does not establish resource
                      health.
                    </p>
                  ) : (
                    conditions.map((condition, i) => (
                      <div
                        key={`${condition.type}-${i}`}
                        className="rounded-control border border-border-default p-3 text-sm"
                      >
                        <p className="font-medium">
                          {condition.type}: {condition.status}
                        </p>
                        <p className="text-text-secondary">
                          {condition.reason}
                          {condition.reason && condition.message ? " — " : ""}
                          {condition.message}
                        </p>
                        <p className="mt-1 text-xs text-text-muted">
                          {conditionFreshness(condition, generation)} · last
                          transition{" "}
                          {condition.last_transition_time ?? "unknown"}
                        </p>
                      </div>
                    ))
                  )}
                </>
              )}
            </>
          )}
          {mode === "edit" && (
            <>
              <p className="text-xs text-text-muted">
                Name, namespace, kind, and API version must remain unchanged.
                Status and managedFields are controller-managed and excluded
                from submission.
              </p>
              <label className="block text-sm">
                Manifest
                <textarea
                  aria-label="Custom resource manifest"
                  value={draft}
                  onChange={(e) => changeDraft(e.target.value)}
                  disabled={busy || !!confirmation || capability.globalReadOnly}
                  spellCheck={false}
                  className="mt-2 min-h-64 w-full rounded-control border border-border-default bg-shell p-3 font-mono text-xs"
                />
              </label>
              <details>
                <summary className="cursor-pointer text-xs text-text-secondary">
                  Version schema
                </summary>
                <pre className="max-h-60 overflow-auto text-xs">
                  {schema
                    ? JSON.stringify(schema, null, 2)
                    : "No schema published."}
                </pre>
              </details>
              {stale && (
                <p role="alert" className="text-warning">
                  The resource changed. Cancel editing and reload before
                  applying.
                </p>
              )}
              {validated && (
                <div className="space-y-2">
                  <p className="text-sm text-success">
                    Server validation passed. Review the effective changes
                    before confirming.
                  </p>
                  <div className="max-h-64 overflow-auto">
                    <YamlDiffView before={baseline} after={validated.yaml} />
                  </div>
                </div>
              )}
            </>
          )}
          {confirmation && (
            <div className="space-y-3 rounded-control border border-warning/40 bg-warning-soft p-3">
              <p className="text-sm">
                {confirmation === "delete"
                  ? "Delete this custom resource? Its controller may delete resources it manages. Finalizers can delay deletion."
                  : `Confirm ${create ? "creation" : "update"} of this resource using the validated manifest. Controllers may act on this change.`}
              </p>
              <label className="block text-sm">
                Type {name} to confirm
                <Input
                  aria-label="Confirmation name"
                  value={typed}
                  onChange={(e) => setTyped(e.target.value)}
                  disabled={busy}
                  className="mt-1"
                />
              </label>
              <div className="flex gap-2">
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() => {
                    setConfirmation(null);
                    setTyped("");
                  }}
                >
                  Cancel confirmation
                </Button>
                <Button
                  variant={
                    confirmation === "delete" ? "destructive" : "default"
                  }
                  disabled={
                    busy ||
                    typed !== name ||
                    !capability.canMutate ||
                    !approvalValid
                  }
                  onClick={() =>
                    void run(confirmation === "delete" ? "delete" : "write")
                  }
                >
                  {confirmation === "delete"
                    ? "Confirm deletion"
                    : create
                      ? "Confirm creation"
                      : "Confirm update"}
                </Button>
              </div>
            </div>
          )}
        </div>
        <div className="flex flex-wrap justify-end gap-2 border-t border-border-default pt-3">
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Close
          </Button>
          {mode === "inspect" && (
            <>
              <Button
                variant="secondary"
                disabled={busy || !!confirmation || detail.isFetching}
                onClick={() => void detail.refetch()}
              >
                Reload
              </Button>
              <Button
                variant="destructive"
                disabled={
                  busy ||
                  !!confirmation ||
                  detail.isFetching ||
                  !capability.canMutate ||
                  deleteAccess.data?.allowed !== true ||
                  !detail.data?.uid
                }
                onClick={() => {
                  requestConfirmation("delete");
                }}
              >
                Delete
              </Button>
              <Button
                disabled={
                  busy ||
                  !!confirmation ||
                  !canWrite ||
                  !detail.data ||
                  detail.isFetching
                }
                onClick={beginEdit}
              >
                Edit YAML
              </Button>
            </>
          )}
          {mode === "edit" && !confirmation && (
            <>
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() => {
                  operation.current++;
                  setMode(create ? "setup" : "inspect");
                  setValidated(null);
                }}
              >
                Cancel editing
              </Button>
              <Button
                variant="secondary"
                disabled={busy || !canWrite || stale}
                onClick={() => void run("validate")}
              >
                {busy ? "Working…" : "Validate on server"}
              </Button>
              <Button
                disabled={
                  busy ||
                  !capability.canMutate ||
                  !canWrite ||
                  stale ||
                  validated?.draft !== draft
                }
                onClick={() => {
                  requestConfirmation("write");
                }}
              >
                {create ? "Create resource" : "Apply changes"}
              </Button>
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
