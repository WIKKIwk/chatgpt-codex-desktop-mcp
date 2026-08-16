use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures::Stream;
use rmcp::transport::streamable_http_server::session::{
    EventStore, RestoreOutcome, ServerSseMessage, SessionId, SessionManager,
    local::{LocalSessionManager, LocalSessionManagerError},
};
use rmcp::{model::ClientJsonRpcMessage, model::ServerJsonRpcMessage};

pub const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
const SESSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Adds the reference server's last-used TTL to rmcp's in-memory sessions.
///
/// rmcp closes an idle worker after `keep_alive`, but leaves its handle in the
/// manager map. The wrapper removes expired IDs before routing a request so a
/// stale session is reported as unknown instead of reaching a dead worker.
pub(crate) struct TtlSessionManager {
    inner: Arc<LocalSessionManager>,
    last_used: tokio::sync::RwLock<HashMap<SessionId, Instant>>,
    ttl: Duration,
}

impl TtlSessionManager {
    pub(crate) fn new(ttl: Duration) -> Arc<Self> {
        let mut inner = LocalSessionManager::default();
        inner.session_config.keep_alive = Some(ttl);
        let manager = Arc::new(Self {
            inner: Arc::new(inner),
            last_used: tokio::sync::RwLock::new(HashMap::new()),
            ttl,
        });
        let weak_manager = Arc::downgrade(&manager);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SESSION_CLEANUP_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(manager) = weak_manager.upgrade() else {
                    break;
                };
                manager.expire_stale().await;
            }
        });
        manager
    }

    async fn expire_stale(&self) {
        let now = Instant::now();
        let expired = {
            let mut last_used = self.last_used.write().await;
            let expired: Vec<_> = last_used
                .iter()
                .filter(|(_, last_used)| now.duration_since(**last_used) > self.ttl)
                .map(|(id, _)| id.clone())
                .collect();
            for id in &expired {
                last_used.remove(id);
            }
            expired
        };

        for id in expired {
            let _ = self.inner.close_session(&id).await;
        }
    }

    async fn touch(&self, id: &SessionId) {
        let mut last_used = self.last_used.write().await;
        last_used.insert(id.clone(), Instant::now());
    }

    async fn touch_active(&self, id: &SessionId) -> Result<(), LocalSessionManagerError> {
        self.expire_stale().await;
        if self.inner.has_session(id).await? {
            self.touch(id).await;
        }
        Ok(())
    }
}

impl SessionManager for TtlSessionManager {
    type Error = LocalSessionManagerError;
    type Transport = <LocalSessionManager as SessionManager>::Transport;

    async fn create_session(&self) -> Result<(SessionId, Self::Transport), Self::Error> {
        self.expire_stale().await;
        let (id, transport) = self.inner.create_session().await?;
        self.touch(&id).await;
        Ok((id, transport))
    }

    async fn initialize_session(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<ServerJsonRpcMessage, Self::Error> {
        self.touch_active(id).await?;
        self.inner.initialize_session(id, message).await
    }

    async fn has_session(&self, id: &SessionId) -> Result<bool, Self::Error> {
        self.expire_stale().await;
        let active = self.inner.has_session(id).await?;
        if active {
            self.touch(id).await;
        } else {
            self.last_used.write().await.remove(id);
        }
        Ok(active)
    }

    async fn close_session(&self, id: &SessionId) -> Result<(), Self::Error> {
        self.last_used.write().await.remove(id);
        self.inner.close_session(id).await
    }

    async fn create_stream(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.touch_active(id).await?;
        self.inner.create_stream(id, message).await
    }

    async fn accept_message(
        &self,
        id: &SessionId,
        message: ClientJsonRpcMessage,
    ) -> Result<(), Self::Error> {
        self.touch_active(id).await?;
        self.inner.accept_message(id, message).await
    }

    async fn create_standalone_stream(
        &self,
        id: &SessionId,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.touch_active(id).await?;
        self.inner.create_standalone_stream(id).await
    }

    async fn resume(
        &self,
        id: &SessionId,
        last_event_id: String,
    ) -> Result<impl Stream<Item = ServerSseMessage> + Send + Sync + 'static, Self::Error> {
        self.touch_active(id).await?;
        self.inner.resume(id, last_event_id).await
    }

    async fn restore_session(
        &self,
        id: SessionId,
    ) -> Result<RestoreOutcome<Self::Transport>, Self::Error> {
        self.expire_stale().await;
        let result = self.inner.restore_session(id.clone()).await?;
        if matches!(
            result,
            RestoreOutcome::Restored(_) | RestoreOutcome::AlreadyPresent
        ) {
            self.touch(&id).await;
        }
        Ok(result)
    }

    fn event_store(&self) -> Option<Arc<dyn EventStore>> {
        self.inner.event_store()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_session_is_removed_before_lookup() {
        let manager = TtlSessionManager::new(SESSION_TTL);
        let (id, _transport) = manager.create_session().await.expect("session");
        {
            let mut last_used = manager.last_used.write().await;
            last_used.insert(
                id.clone(),
                Instant::now() - SESSION_TTL - Duration::from_secs(1),
            );
        }

        assert!(!manager.has_session(&id).await.expect("session lookup"));
        assert!(manager.last_used.read().await.is_empty());
    }
}
