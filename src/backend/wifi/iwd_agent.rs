//! iwd Agent implementation for password prompts
//!
//! This module implements the net.connman.iwd.Agent D-Bus interface.
//! iwd calls our agent when it needs credentials (e.g., WiFi password).

use async_channel::Sender;
use tokio::sync::oneshot;
use zbus::interface;
use zbus::zvariant::ObjectPath;

use super::iwd_proxy::NetworkProxy;


/// Request sent from Agent to backend for passphrase input
pub struct PassphraseRequest {
    pub network_path: String,
    pub network_name: String,
    pub response_tx: oneshot::Sender<Option<String>>,
}

/// iwd Agent that handles credential requests
///
/// When iwd needs a password, it calls RequestPassphrase on our agent.
/// We send a request to the UI, wait for the user to enter the password,
/// and return it to iwd.
pub struct IwdAgent {
    /// Channel to send passphrase requests to the main backend loop
    request_tx: Sender<PassphraseRequest>,
}

impl IwdAgent {
    pub fn new(request_tx: Sender<PassphraseRequest>) -> Self {
        Self { request_tx }
    }
}

#[interface(name = "net.connman.iwd.Agent")]
impl IwdAgent {
    /// Called by iwd when it needs a passphrase for a network
    async fn request_passphrase(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        network: ObjectPath<'_>,
    ) -> zbus::fdo::Result<String> {
        let network_path = network.to_string();
        tracing::info!("iwd requesting passphrase for {}", network_path);

        // Get network name from iwd
        let network_name = match NetworkProxy::builder(conn)
            .path(network.clone())
            .ok()
            .map(|b| b.build())
        {
            Some(fut) => match fut.await {
                Ok(proxy) => proxy.name().await.unwrap_or_else(|_| "Unknown".into()),
                Err(_) => "Unknown".into(),
            },
            None => "Unknown".into(),
        };

        // Create oneshot channel for response
        let (response_tx, response_rx) = oneshot::channel();

        let request = PassphraseRequest {
            network_path: network_path.clone(),
            network_name,
            response_tx,
        };

        if self.request_tx.send(request).await.is_err() {
            tracing::error!("Failed to send passphrase request to backend");
            return Err(zbus::fdo::Error::Failed("Backend channel closed".into()));
        }

        // Wait for response from UI
        match response_rx.await {
            Ok(Some(passphrase)) => {
                tracing::info!("Got passphrase for {}", network_path);
                Ok(passphrase)
            }
            Ok(None) => {
                tracing::info!("User cancelled passphrase entry for {}", network_path);
                // iwd treats any error from agent as cancellation
                Err(zbus::fdo::Error::AuthFailed("User cancelled".into()))
            }
            Err(_) => {
                tracing::error!("Passphrase response channel closed");
                Err(zbus::fdo::Error::Failed("Response channel closed".into()))
            }
        }
    }

    /// Called by iwd when authentication was cancelled
    async fn cancel(&self, reason: String) -> zbus::fdo::Result<()> {
        tracing::info!("iwd cancelled agent request: {}", reason);
        Ok(())
    }

    /// Called when the agent is being released
    async fn release(&self) -> zbus::fdo::Result<()> {
        tracing::info!("iwd agent released");
        Ok(())
    }
}
