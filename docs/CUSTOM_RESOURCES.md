# Custom resources and Helm repositories

## Custom-resource explorer

Open **CRDs** in a cluster workspace, choose a definition, then select a served
API version and namespace. Namespaced resources support the shared namespace
picker, including entering a known namespace when namespace discovery is denied.
Cluster-scoped resources do not use a namespace.

The table shows the CRD's additional printer columns. Enable **Additional
columns** to show columns with nonzero priority. Simple dotted JSONPath fields
are supported; more complex expressions are marked **Unsupported path** and
remain inspectable in YAML. Missing values are shown separately from unsupported
expressions. Filter instances by name, namespace, or reported status.

Open an instance to inspect its YAML, schema, UID, generation, and controller
conditions, including reasons, messages, transition times, and observed-generation
freshness. Missing conditions are not interpreted as healthy. Refresh lists or
reload the inspector to obtain a new snapshot; this view does not watch changes.

### Create, update, and delete instances

- **Create resource** requires an explicit name and, for namespaced types, a
  namespace. Fill in the manifest using the published version schema. The server
  validates required fields, admission, and authorization. Creation uses POST:
  an existing object with the same name is not overwritten.
- **Edit YAML** preserves the selected identity and the loaded UID/resourceVersion.
  Updates use server-side apply without force ownership. Status and managedFields
  are excluded from submission. Changes to these controller-managed fields in the
  editor do not update them.
- Both operations require **Validate on server**, review of the effective result,
  and typed confirmation. Editing the draft invalidates validation. A locked
  context permits previews but blocks real writes; global read-only mode disables
  editing. Mutation permissions are checked against the actual API group/resource.
- **Delete** requires delete permission and typed confirmation. UID and revision
  preconditions prevent deleting a replacement or changed object. Finalizers may
  delay completion; successful deletion requests do not imply finalizer completion.
- If the object changed, reload and review it again. Lumen does not force field
  conflicts or retry stale changes. Changes of context, namespace, version, or
  selected definition close the previous inspector and discard its draft.

This explorer manages **custom-resource instances**. CRD definitions are inspected
for versions and schemas; creating, changing, or deleting CRD definitions is not
part of this workflow. CRD list/get access is currently required for discovery
and validating each dynamic resource target. CR-only RBAC access without CRD
read permission is not sufficient for this explorer.

## Deploying charts from Helm repositories

Open **Helm → install → Manage Helm repositories**. Add a named HTTP(S)
repository, refresh indexes, search for a chart, choose a version, customize
values, then use the existing dry-run and installation review flow.

Repository settings belong to the local Helm CLI and are shared across all
clusters on the machine. Removing a repository only removes that local entry;
it does not uninstall releases. Repository operations have a 60-second process
timeout and are serialized within Lumen. A refresh with a failed repository
reports an error even if other repositories updated successfully.

The Helm CLI must be installed. Existing authenticated repositories remain
available; configure authentication, custom certificates, or other private-repo
settings through the Helm CLI. The add form accepts URLs without embedded
credentials, query parameters, or fragments. Existing displayed URLs omit
credentials and queries. OCI references can be entered directly in the chart
field; OCI registry authentication also uses the local Helm configuration.

Repository configuration changes do not modify cluster resources. Installing or
upgrading a chart retains Lumen's existing target-context protection and review
flow. Adding a repository does not install any chart automatically.
