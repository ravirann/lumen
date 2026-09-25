//! Multi-cluster client manager.
//!
//! Each kubeconfig context gets its own `kube::Client`, created lazily on first
//! use and cached. A single frontend session may hold clients for many
//! contexts simultaneously (fleet mode). `current_context` is kept for legacy
//! single-cluster commands; all new commands accept an explicit `context` arg.

use crate::error::{AppError, AppResult};
use crate::k8s::kubeconfig::{self, ConfigSnapshot};
use kube::{Client, Config};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

struct CachedClient {
    revision: kubeconfig::sources::SourceRevision,
    client: Client,
}

#[derive(Default)]
pub struct K8sState {
    pub current_context: RwLock<Option<String>>,
    /// Cache of `Client` keyed by context name. Populated lazily.
    clients: RwLock<HashMap<String, CachedClient>>,
    /// Credential setup for one context must not block connections to other clusters.
    client_builds: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    cache_epoch: AtomicU64,
    /// Cancellation tokens for running streams, keyed by stream id.
    pub streams: RwLock<HashMap<String, CancellationToken>>,
}

impl K8sState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Build (or reuse) a Client for the given kubeconfig context.
    async fn build_client(context: &str, kc: kube::config::Kubeconfig) -> AppResult<Client> {
        let opts = kube::config::KubeConfigOptions {
            context: Some(context.to_string()),
            ..Default::default()
        };
        let cfg = Config::from_custom_kubeconfig(kc, &opts)
            .await
            .map_err(|e| AppError::Kubeconfig(e.to_string()))?;
        Client::try_from(cfg).map_err(|e| AppError::K8s(e.to_string()))
    }

    /// Get (or create + cache) a Client for `context`.
    pub async fn client_for(&self, context: &str) -> AppResult<Client> {
        self.client_for_loaded(
            context,
            || kubeconfig::load_snapshot().map(ConfigSnapshot::track_credentials),
            |config| Self::build_client(context, config),
        )
        .await
    }

    async fn client_for_loaded<L, B, F>(
        &self,
        context: &str,
        load: L,
        build: B,
    ) -> AppResult<Client>
    where
        L: Fn() -> AppResult<ConfigSnapshot>,
        B: Fn(kube::config::Kubeconfig) -> F,
        F: std::future::Future<Output = AppResult<Client>>,
    {
        let gate = self
            .client_builds
            .lock()
            .await
            .entry(context.to_owned())
            .or_default()
            .clone();
        let _build = gate.lock().await;
        for _ in 0..3 {
            let epoch = self.cache_epoch.load(Ordering::SeqCst);
            let snapshot = load()?;
            if let Some(cached) = self.clients.read().await.get(context) {
                if cached.revision == snapshot.revision {
                    return Ok(cached.client.clone());
                }
            }
            let client = build(snapshot.config.clone()).await?;
            // Re-load after asynchronous credential setup. Environment and source contents may
            // have changed while the client was being built; do not publish that stale client.
            if load()?.revision != snapshot.revision {
                continue;
            }
            let mut clients = self.clients.write().await;
            if self.cache_epoch.load(Ordering::SeqCst) != epoch {
                continue;
            }
            clients.insert(
                context.to_owned(),
                CachedClient {
                    revision: snapshot.revision,
                    client: client.clone(),
                },
            );
            return Ok(client);
        }
        Err(AppError::Kubeconfig(
            "kubeconfig changed while connecting; retry the connection".into(),
        ))
    }

    /// Set the "active" context — legacy single-cluster convenience. Also
    /// pre-warms the client cache for that context.
    pub async fn set_context(&self, name: &str) -> AppResult<()> {
        let _ = self.client_for(name).await?;
        *self.current_context.write().await = Some(name.to_string());
        Ok(())
    }

    /// Resolve the context to use: explicit override, else the legacy current.
    pub async fn resolve_context(&self, explicit: Option<&str>) -> AppResult<String> {
        if let Some(c) = explicit {
            return Ok(c.to_string());
        }
        self.current_context
            .read()
            .await
            .clone()
            .ok_or_else(|| AppError::K8s("no active cluster context".into()))
    }

    /// Legacy: client for the current context. New code should prefer
    /// `client_for(ctx)`.
    pub async fn client(&self) -> AppResult<Client> {
        let ctx = self.resolve_context(None).await?;
        self.client_for(&ctx).await
    }

    /// Evict a cached client (e.g. on auth failure).
    pub async fn invalidate(&self, context: &str) {
        let mut clients = self.clients.write().await;
        self.cache_epoch.fetch_add(1, Ordering::SeqCst);
        clients.remove(context);
    }

    /// Evict *all* cached clients. Useful when the kubeconfig changes on disk
    /// or the user explicitly wants to force a reconnect across every context.
    pub async fn invalidate_all(&self) {
        let mut clients = self.clients.write().await;
        self.cache_epoch.fetch_add(1, Ordering::SeqCst);
        clients.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture(name: &str, server: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("lumen-client-{}-{name}", std::process::id()));
        write(&path, server);
        path
    }

    fn write(path: &std::path::Path, server: &str) {
        std::fs::write(path, format!("contexts: [{{name: dev, context: {{cluster: c}}}}]\nclusters: [{{name: c, cluster: {{server: {server}}}}}]\n")).unwrap();
    }

    #[tokio::test]
    async fn client_negotiates_and_decodes_compressed_responses() {
        use std::io::Write;
        use wiremock::{
            matchers::{header, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let body = r#"{"major":"1","minor":"33","gitVersion":"v1.33.8","gitCommit":"test","gitTreeState":"clean","buildDate":"2026-01-01T00:00:00Z","goVersion":"go1.24","compiler":"gc","platform":"linux/amd64"}"#;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(body.as_bytes()).unwrap();
        Mock::given(path("/version"))
            .and(header("accept-encoding", "gzip"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", "gzip")
                    .set_body_bytes(gzip.finish().unwrap()),
            )
            .mount(&server)
            .await;
        let config: kube::config::Kubeconfig = serde_yaml::from_str(&format!("contexts: [{{name: dev, context: {{cluster: c}}}}]\nclusters: [{{name: c, cluster: {{server: {}}}}}]\n", server.uri())).unwrap();
        let client = K8sState::build_client("dev", config).await.unwrap();
        assert_eq!(
            client.apiserver_version().await.unwrap().git_version,
            "v1.33.8"
        );

        // A user's explicit kubeconfig opt-out still takes precedence.
        server.reset().await;
        Mock::given(path("/version"))
            .and(|request: &wiremock::Request| !request.headers.contains_key("accept-encoding"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let config: kube::config::Kubeconfig = serde_yaml::from_str(&format!("contexts: [{{name: dev, context: {{cluster: c}}}}]\nclusters: [{{name: c, cluster: {{server: {}, disable-compression: true}}}}]\n", server.uri())).unwrap();
        let client = K8sState::build_client("dev", config).await.unwrap();
        assert_eq!(
            client.apiserver_version().await.unwrap().git_version,
            "v1.33.8"
        );
    }

    #[tokio::test]
    async fn blocked_context_does_not_block_cached_peer_and_invalidation_discards_build() {
        let path = fixture("concurrency", "http://127.0.0.1:10001");
        let state = K8sState::default();
        let load = || kubeconfig::load_paths(std::slice::from_ref(&path));
        state
            .client_for_loaded("peer", load, |config| K8sState::build_client("dev", config))
            .await
            .unwrap();
        let entered = tokio::sync::Notify::new();
        let release = tokio::sync::Notify::new();
        let builds = AtomicUsize::new(0);
        let slow = state.client_for_loaded("slow", load, |config| {
            let entered = &entered;
            let release = &release;
            let builds = &builds;
            async move {
                if builds.fetch_add(1, Ordering::SeqCst) == 0 {
                    entered.notify_one();
                    release.notified().await;
                }
                K8sState::build_client("dev", config).await
            }
        });
        let other = async {
            entered.notified().await;
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                state.client_for_loaded("peer", load, |config| {
                    K8sState::build_client("dev", config)
                }),
            )
            .await
            .expect("cached peer blocked by another context")
            .unwrap();
            state.invalidate_all().await;
            release.notify_one();
        };
        let (result, ()) = tokio::join!(slow, other);
        result.unwrap();
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "invalidation must discard the in-flight client"
        );
    }

    #[tokio::test]
    async fn source_changes_rebuild_and_old_in_flight_build_cannot_repopulate_cache() {
        let path = fixture("race", "http://127.0.0.1:10001");
        let state = K8sState::default();
        let builds = AtomicUsize::new(0);
        let load = || kubeconfig::load_paths(std::slice::from_ref(&path));
        let build = |config| {
            if builds.fetch_add(1, Ordering::SeqCst) == 0 {
                write(&path, "http://127.0.0.1:10002");
            }
            K8sState::build_client("dev", config)
        };
        state.client_for_loaded("dev", load, build).await.unwrap();
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "changed source must discard the first build"
        );
        state.client_for_loaded("dev", load, build).await.unwrap();
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "unchanged sources reuse cache"
        );
        write(&path, "http://127.0.0.1:10003");
        state.client_for_loaded("dev", load, build).await.unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 3);
        state.invalidate("dev").await;
        state.client_for_loaded("dev", load, build).await.unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 4);
    }
}
