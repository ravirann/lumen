//! Custom-resource IPCs. Mutation context/protection is enforced natively.
use crate::{
    error::AppResult,
    k8s::{actions::ApplyOutcome, crd, rbac::AccessReviewResult},
    state::AppState,
};
use tauri::State;

#[tauri::command]
pub async fn get_crd_details(
    crd_name: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<crd::CrdDetails> {
    let client = super::k8s::client_for(&state, context.as_deref()).await?;
    crd::get_details(&client, &crd_name).await
}
#[tauri::command]
pub async fn list_custom_resources(
    crd_name: String,
    version: String,
    namespace: Option<String>,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<crd::CrList> {
    let client = super::k8s::client_for(&state, context.as_deref()).await?;
    crd::list_custom_resources(
        &client,
        &crd::CrTarget {
            crd_name,
            version,
            namespace,
            name: String::new(),
            uid: None,
            resource_version: None,
        },
    )
    .await
}
#[tauri::command]
pub async fn get_custom_resource(
    target: crd::CrTarget,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<crd::CrDetail> {
    let client = super::k8s::client_for(&state, context.as_deref()).await?;
    crd::get_custom_resource(&client, &target).await
}
#[tauri::command]
pub async fn check_cr_access(
    target: crd::CrTarget,
    verb: String,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<AccessReviewResult> {
    let client = super::k8s::client_for(&state, context.as_deref()).await?;
    crd::check_access(&client, &target, &verb).await
}
#[tauri::command]
pub async fn write_cr(
    target: crd::CrTarget,
    yaml: String,
    create: bool,
    dry_run: bool,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ApplyOutcome> {
    let (ctx, client, identity, _) =
        super::k8s::mutation_client(&state, context.as_deref(), dry_run).await?;
    crd::write_cr(&client, &target, &yaml, create, dry_run, || {
        super::k8s::require_current_target(&state.protection, &ctx, &identity, dry_run)
    })
    .await
}
#[tauri::command]
pub async fn delete_cr(
    target: crd::CrTarget,
    context: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let (ctx, client, identity, _) =
        super::k8s::mutation_client(&state, context.as_deref(), false).await?;
    crd::delete_cr(&client, &target, || {
        super::k8s::require_current_target(&state.protection, &ctx, &identity, false)
    })
    .await
}
