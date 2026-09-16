import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { helmRepositories } from "@/lib/helmRepositories";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorMessage } from "@/lib/errorMessage";

export function HelmRepositories() {
  const client = useQueryClient();
  const repositories = useQuery({
    queryKey: ["helm-repositories"],
    queryFn: helmRepositories.list,
  });
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [removeName, setRemoveName] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const mutation = useMutation({
    mutationFn: async (action: "add" | "update" | "remove") => {
      setMessage("");
      if (action === "add") await helmRepositories.add(name.trim(), url.trim());
      else if (action === "remove" && removeName)
        await helmRepositories.remove(removeName);
      else if (action === "update") await helmRepositories.update();
      return action;
    },
    onSuccess: async (action) => {
      if (action === "add") {
        setName("");
        setUrl("");
      }
      setRemoveName(null);
      setMessage(
        action === "update"
          ? "Repository indexes refreshed."
          : action === "add"
            ? "Repository added."
            : "Repository removed. Installed releases are unchanged.",
      );
      await Promise.all([
        client.invalidateQueries({ queryKey: ["helm-repositories"] }),
        client.invalidateQueries({ queryKey: ["helm-search"] }),
      ]);
    },
  });
  const error = mutation.error ?? repositories.error;

  return (
    <details className="rounded border border-border-subtle p-3">
      <summary className="cursor-pointer text-[12px] text-text-primary">
        Manage Helm repositories
      </summary>
      <div className="mt-3 flex flex-col gap-3">
        <p className="text-[11px] text-text-muted">
          Changes affect your local Helm configuration for all clusters on this
          machine. Authenticated repositories configured through the Helm CLI
          remain available. Add HTTP(S) chart repositories here; enter OCI chart
          references directly below.
        </p>
        {repositories.isPending && (
          <p role="status" className="text-[11px] text-text-muted">
            Loading repositories…
          </p>
        )}
        {error && (
          <p role="alert" className="text-[11px] text-error">
            {errorMessage(error)}
          </p>
        )}
        {message && (
          <p role="status" className="text-[11px] text-text-secondary">
            {message}
          </p>
        )}
        {repositories.data?.length === 0 && (
          <p className="text-[11px] text-text-muted">
            No repositories configured. Add one to search its charts.
          </p>
        )}
        <ul className="space-y-2">
          {repositories.data?.map((repo) => (
            <li key={repo.name} className="flex items-center gap-2 text-[11px]">
              <div className="min-w-0 flex-1">
                <span className="font-mono text-text-primary">{repo.name}</span>
                <p className="break-all text-text-muted">{repo.url}</p>
              </div>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={mutation.isPending}
                aria-label={`Remove repository ${repo.name}`}
                onClick={() => {
                  setRemoveName(repo.name);
                  mutation.reset();
                }}
              >
                remove
              </Button>
            </li>
          ))}
        </ul>
        {removeName && (
          <div className="rounded border border-warning/40 p-2 text-[11px] text-text-secondary">
            <p>
              Remove {removeName} from local Helm configuration? Installed
              releases stay in the cluster.
            </p>
            <div className="mt-2 flex gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={mutation.isPending}
                onClick={() => setRemoveName(null)}
              >
                cancel
              </Button>
              <Button
                type="button"
                size="sm"
                disabled={mutation.isPending}
                onClick={() => mutation.mutate("remove")}
              >
                confirm removal
              </Button>
            </div>
          </div>
        )}
        <form
          className="flex flex-col gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            mutation.mutate("add");
          }}
        >
          <label className="text-[11px] text-text-muted">
            Repository name
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-charts"
              required
              pattern="[A-Za-z0-9][A-Za-z0-9_.-]*"
              maxLength={128}
              disabled={mutation.isPending}
            />
          </label>
          <label className="text-[11px] text-text-muted">
            Repository URL
            <Input
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://charts.example.org"
              required
              disabled={mutation.isPending}
            />
          </label>
          <p className="text-[11px] text-text-muted">
            Do not include credentials or query parameters. Configure private
            repository authentication using your local Helm CLI.
          </p>
          <div className="flex gap-2">
            <Button
              type="submit"
              size="sm"
              variant="outline"
              disabled={mutation.isPending || !name.trim() || !url.trim()}
            >
              add repository
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={mutation.isPending || !repositories.data?.length}
              onClick={() => mutation.mutate("update")}
            >
              refresh indexes
            </Button>
          </div>
          {mutation.isPending && (
            <p role="status" className="text-[11px] text-text-muted">
              Updating local Helm repositories…
            </p>
          )}
        </form>
      </div>
    </details>
  );
}
