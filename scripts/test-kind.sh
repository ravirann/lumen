#!/usr/bin/env bash
set -euo pipefail
# Never use or modify the user's kubeconfig or currently selected context.
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
kind_bin="${LUMEN_KIND_BIN:-kind}"
command -v "$kind_bin" >/dev/null
command -v kubectl >/dev/null
fixture_dir="$(mktemp -d)"
cluster="lumen-e2e-$(date +%s)-$$"
cleanup() {
  "$kind_bin" delete cluster --name "$cluster"
  rm -rf "$fixture_dir"
}
trap cleanup EXIT
export KUBECONFIG="$fixture_dir/config"
export LUMEN_E2E_KUBECONFIG="$KUBECONFIG"
"$kind_bin" create cluster --name "$cluster" --kubeconfig "$KUBECONFIG" --wait 120s
kubectl --context "kind-$cluster" apply -f - <<'YAML'
apiVersion: v1
kind: Namespace
metadata:
  name: lumen-e2e-a
---
apiVersion: v1
kind: Namespace
metadata:
  name: lumen-e2e-b
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: viewer
  namespace: lumen-e2e-a
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: viewer
  namespace: lumen-e2e-a
rules:
  - apiGroups: [""]
    resources: ["configmaps"]
    verbs: ["get", "list"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: viewer
  namespace: lumen-e2e-a
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: viewer
subjects:
  - kind: ServiceAccount
    name: viewer
    namespace: lumen-e2e-a
YAML
cargo test --manifest-path "$root/src-tauri/Cargo.toml" --locked --test release_cluster --test debug_cluster --test devices_cluster --test gpu_cluster --test kueue_cluster -- --ignored --nocapture
