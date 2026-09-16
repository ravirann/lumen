pub mod commands;
pub mod error;
pub mod k8s;
pub mod protection;
pub mod state;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // kube_client logs every failed TCP connect at ERROR, which
                // floods the console when a fleet context is unreachable.
                // We surface those failures through our own AppError path, so
                // silence the crate-level spam by default. Users can still set
                // RUST_LOG explicitly to debug.
                tracing_subscriber::EnvFilter::new(
                    "info,kube_client::client::builder=off,kube_client::client=error",
                )
            }),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(AppState::with_config_dir(app.path().app_config_dir()?));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connection::diagnose_connection,
            commands::devices::device_resources_snapshot,
            commands::k8s::get_context_protection,
            commands::k8s::set_context_protection,
            commands::k8s::unlock_context,
            commands::k8s::lock_context,
            commands::k8s::list_contexts,
            commands::k8s::set_context,
            commands::k8s::delete_context,
            commands::k8s::list_deleted_contexts,
            commands::k8s::restore_deleted_context,
            commands::k8s::list_namespaces,
            commands::k8s::list_workloads,
            commands::k8s::get_resource,
            commands::k8s::get_rbac_details,
            commands::k8s::get_storage_details,
            commands::k8s::get_resource_insights,
            commands::k8s::stream_events,
            commands::k8s::stop_stream,
            commands::k8s::watch_nodes,
            commands::k8s::watch_workloads,
            commands::k8s::stream_logs,
            commands::k8s::capture_incident_logs,
            commands::k8s::list_pods_for,
            commands::k8s::list_pods_on_node,
            commands::k8s::list_fleet,
            commands::k8s::probe_fleet_context,
            commands::k8s::disconnect_context,
            commands::k8s::reconnect_all,
            commands::k8s::list_nodes,
            commands::k8s::metrics_explorer_snapshot,
            commands::k8s::cloud_map,
            commands::k8s::network_debug_snapshot,
            commands::k8s::security_scan,
            commands::k8s::check_access,
            commands::k8s::list_crds,
            commands::crd::get_crd_details,
            commands::crd::list_custom_resources,
            commands::crd::get_custom_resource,
            commands::crd::check_cr_access,
            commands::crd::write_cr,
            commands::crd::delete_cr,
            commands::helm_repositories::helm_repository_list,
            commands::helm_repositories::helm_repository_add,
            commands::helm_repositories::helm_repository_remove,
            commands::helm_repositories::helm_repository_update,
            commands::k8s::list_cr_instances,
            commands::k8s::get_cr_yaml,
            commands::k8s::provision_team_access,
            commands::k8s::revoke_team_access,
            commands::k8s::list_team_access,
            commands::k8s::renew_team_token,
            commands::k8s::rotate_team_token,
            commands::k8s::list_events_for,
            commands::k8s::restart_workload,
            commands::k8s::scale_workload,
            commands::k8s::set_workload_image,
            commands::k8s::list_manual_cronjob_runs,
            commands::k8s::detect_trivy,
            commands::k8s::scan_image,
            commands::k8s::delete_pod,
            commands::k8s::cordon_node,
            commands::k8s::uncordon_node,
            commands::k8s::drain_node,
            commands::k8s::delete_resource,
            commands::k8s::trigger_cronjob,
            commands::k8s::start_port_forward,
            commands::k8s::list_port_forwards,
            commands::k8s::stop_port_forward,
            commands::k8s::list_pod_containers,
            commands::k8s::apply_resource,
            commands::k8s::list_helm_releases,
            commands::k8s::get_helm_release,
            commands::k8s::list_helm_history,
            commands::k8s::helm_install,
            commands::k8s::helm_upgrade,
            commands::k8s::helm_rollback,
            commands::k8s::helm_uninstall,
            commands::k8s::helm_search_repo,
            commands::k8s::helm_show_values,
            commands::k8s::get_debug_target,
            commands::k8s::create_debug_container,
            commands::k8s::start_pod_attach,
            commands::k8s::pod_attach_stdin,
            commands::k8s::pod_attach_resize,
            commands::k8s::pod_attach_close,
            commands::k8s::get_pod_details,
            commands::k8s::detect_argocd,
            commands::k8s::list_argocd_applications,
            commands::k8s::get_argocd_application,
            commands::k8s::sync_argocd_application,
            commands::k8s::refresh_argocd_application,
            commands::k8s::terminate_argocd_operation,
            commands::k8s::detect_argocd_application_sets,
            commands::k8s::list_argocd_application_sets,
            commands::k8s::get_argocd_application_set,
            commands::k8s::list_argocd_app_projects,
            commands::k8s::get_argocd_app_project,
            commands::k8s::detect_tekton,
            commands::k8s::list_pipeline_runs,
            commands::k8s::get_pipeline_run,
            commands::k8s::cancel_pipeline_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running lumen");
}
