use async_channel::Sender;
use bluer::agent::{Agent, AgentHandle, ReqError, ReqResult};
use bluer::{Adapter, AdapterEvent, AdapterProperty, Address, Device, DeviceEvent, DeviceProperty, Session};
use futures::stream::{SelectAll, StreamExt};
use std::collections::HashSet;
use std::pin::Pin;

use super::super::manager::{BackendEvent, BtDeviceData};

/// Convert a bluer error to a user-friendly message
fn format_bt_error(e: &bluer::Error) -> String {
    let s = e.to_string();
    if s.contains("page-timeout") || s.contains("abort-by-local") {
        "Device not responding. Make sure it is turned on and nearby.".into()
    } else if s.contains("profile-unavailable") {
        "No compatible services found on the device.".into()
    } else if s.contains("already-connected") {
        "Already connected.".into()
    } else if s.contains("connection-timeout") || s.contains("connection-attempt-failed") {
        "Connection timed out.".into()
    } else if s.contains("connection-refused") {
        "Connection refused by the device.".into()
    } else if s.contains("aborted-by-remote") || s.contains("ECONNRESET") {
        "Device disconnected or turned off.".into()
    } else if s.contains("not-powered") {
        "Bluetooth adapter is not powered on.".into()
    } else if s.contains("not-supported") || s.contains("EOPNOTSUPP") {
        "Operation not supported.".into()
    } else if s.contains("busy") || s.contains("EBUSY") || s.contains("in-progress") {
        "Device is busy, try again.".into()
    } else if s.contains("not-ready") {
        "Bluetooth is not ready.".into()
    } else if s.contains("rejected") || s.contains("canceled") {
        "Operation cancelled.".into()
    } else if s.contains("not-paired") || s.contains("not paired") {
        "Device is not paired. Pair first.".into()
    } else if s.contains("authentication") || s.contains("auth") {
        "Authentication failed.".into()
    } else {
        format!("Bluetooth error: {}", s)
    }
}

/// Internal request for pairing interaction, sent from Agent callback to main loop
pub enum BtPairingRequest {
    /// Show passkey and ask user to confirm (Yes/No)
    ConfirmPasskey {
        address: Address,
        passkey: u32,
        response_tx: tokio::sync::oneshot::Sender<ReqResult<()>>,
    },
    /// Ask user to enter a PIN code (returns String)
    RequestPinCode {
        address: Address,
        response_tx: tokio::sync::oneshot::Sender<ReqResult<String>>,
    },
    /// Ask user to enter a numeric passkey (returns u32)
    RequestPasskey {
        address: Address,
        response_tx: tokio::sync::oneshot::Sender<ReqResult<u32>>,
    },
    /// Display a passkey the user should type on the remote device
    DisplayPasskey {
        address: Address,
        passkey: u32,
    },
    /// Display a PIN the user should type on the remote device
    DisplayPinCode {
        address: Address,
        pin_code: String,
    },
    /// Authorize connection (no code, just confirm)
    RequestAuthorization {
        address: Address,
        response_tx: tokio::sync::oneshot::Sender<ReqResult<()>>,
    },
}

/// Type alias for the boxed discovery stream
pub type BtDiscoveryStream = Pin<Box<dyn futures::Stream<Item = AdapterEvent> + Send>>;

/// Type alias for the always-on adapter event stream (not tied to discovery)
pub type BtAdapterEventStream = Pin<Box<dyn futures::Stream<Item = AdapterEvent> + Send>>;

/// Type alias for a single device's event stream tagged with its address
pub type BtDeviceEventStream =
    Pin<Box<dyn futures::Stream<Item = (Address, DeviceEvent)> + Send>>;

pub struct BluetoothBackend {
    adapter: Option<Adapter>,
    evt_tx: Sender<BackendEvent>,
    _agent_handle: Option<AgentHandle>,
}

impl BluetoothBackend {
    /// Create and initialize. Returns the backend + pairing request receiver.
    pub async fn new(
        evt_tx: Sender<BackendEvent>,
    ) -> Result<
        (Self, async_channel::Receiver<BtPairingRequest>),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let session = Session::new().await?;

        // Set up pairing agent
        let (pairing_tx, pairing_rx) = async_channel::unbounded::<BtPairingRequest>();
        let agent = Self::create_agent(pairing_tx);
        let agent_handle = session.register_agent(agent).await?;

        // Get default adapter (may not exist)
        let adapter = match session.default_adapter().await {
            Ok(a) => {
                tracing::info!("Bluetooth adapter: {}", a.name());
                Some(a)
            }
            Err(e) => {
                tracing::warn!("No Bluetooth adapter found: {}. BT features disabled.", e);
                let _ = evt_tx
                    .send(BackendEvent::BtError(format!(
                        "No Bluetooth adapter: {}",
                        e
                    )))
                    .await;
                None
            }
        };

        let backend = Self {
            adapter,
            evt_tx,
            _agent_handle: Some(agent_handle),
        };

        Ok((backend, pairing_rx))
    }

    fn create_agent(pairing_tx: async_channel::Sender<BtPairingRequest>) -> Agent {
        let tx1 = pairing_tx.clone();
        let tx2 = pairing_tx.clone();
        let tx3 = pairing_tx.clone();
        let tx4 = pairing_tx.clone();
        let tx5 = pairing_tx.clone();
        Agent {
            request_default: true,
            request_confirmation: Some(Box::new(move |req| {
                let tx = tx1.clone();
                Box::pin(async move {
                    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                    if tx.send(BtPairingRequest::ConfirmPasskey {
                        address: req.device,
                        passkey: req.passkey,
                        response_tx,
                    }).await.is_err() {
                        return Err(ReqError::Rejected);
                    }
                    response_rx.await.unwrap_or(Err(ReqError::Rejected))
                })
            })),
            request_pin_code: Some(Box::new(move |req| {
                let tx = tx2.clone();
                Box::pin(async move {
                    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                    if tx.send(BtPairingRequest::RequestPinCode {
                        address: req.device,
                        response_tx,
                    }).await.is_err() {
                        return Err(ReqError::Rejected);
                    }
                    response_rx.await.unwrap_or(Err(ReqError::Rejected))
                })
            })),
            request_passkey: Some(Box::new(move |req| {
                let tx = tx3.clone();
                Box::pin(async move {
                    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                    if tx.send(BtPairingRequest::RequestPasskey {
                        address: req.device,
                        response_tx,
                    }).await.is_err() {
                        return Err(ReqError::Rejected);
                    }
                    response_rx.await.unwrap_or(Err(ReqError::Rejected))
                })
            })),
            display_passkey: Some(Box::new(move |req| {
                let tx = tx4.clone();
                Box::pin(async move {
                    let _ = tx.send(BtPairingRequest::DisplayPasskey {
                        address: req.device,
                        passkey: req.passkey,
                    }).await;
                    Ok(())
                })
            })),
            display_pin_code: Some(Box::new(move |req| {
                let tx = tx5.clone();
                Box::pin(async move {
                    let _ = tx.send(BtPairingRequest::DisplayPinCode {
                        address: req.device,
                        pin_code: req.pincode,
                    }).await;
                    Ok(())
                })
            })),
            request_authorization: Some(Box::new(move |req| {
                let tx = pairing_tx.clone();
                Box::pin(async move {
                    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
                    if tx.send(BtPairingRequest::RequestAuthorization {
                        address: req.device,
                        response_tx,
                    }).await.is_err() {
                        return Err(ReqError::Rejected);
                    }
                    response_rx.await.unwrap_or(Err(ReqError::Rejected))
                })
            })),
            ..Default::default()
        }
    }

    /// Send initial adapter state and return streams for already-known devices.
    pub async fn send_initial_state(
        &self,
        device_events: &mut SelectAll<BtDeviceEventStream>,
        tracked_devices: &mut HashSet<Address>,
    ) {
        let Some(ref adapter) = self.adapter else {
            return;
        };

        if let Ok(powered) = adapter.is_powered().await {
            let _ = self.evt_tx.send(BackendEvent::BtPowered(powered)).await;
        }

        if let Ok(discoverable) = adapter.is_discoverable().await {
            let _ = self
                .evt_tx
                .send(BackendEvent::BtDiscoverable(discoverable))
                .await;
        }

        // Send already-paired/connected devices
        if let Ok(addrs) = adapter.device_addresses().await {
            for addr in addrs {
                if let Ok(device) = adapter.device(addr) {
                    if let Some(data) = Self::read_device_data(&device).await {
                        if data.paired || data.connected {
                            Self::start_tracking_device(
                                addr,
                                &device,
                                device_events,
                                tracked_devices,
                            )
                            .await;
                            let _ =
                                self.evt_tx.send(BackendEvent::BtDeviceAdded(data)).await;
                        }
                    }
                }
            }
        }
    }

    /// Start a persistent adapter event stream (DeviceAdded/DeviceRemoved/PropertyChanged).
    /// Does NOT start discovery — only monitors D-Bus signals.
    pub async fn adapter_events(&self) -> Option<BtAdapterEventStream> {
        let adapter = self.adapter.as_ref()?;
        match adapter.events().await {
            Ok(stream) => {
                tracing::info!("Started always-on adapter event stream");
                Some(Box::pin(stream))
            }
            Err(e) => {
                tracing::error!("Failed to start adapter event stream: {}", e);
                None
            }
        }
    }

    /// Read all properties from a bluer::Device into a BtDeviceData
    async fn read_device_data(device: &Device) -> Option<BtDeviceData> {
        Some(BtDeviceData {
            address: device.address().to_string(),
            name: device.name().await.ok().flatten().unwrap_or_default(),
            alias: device.alias().await.ok().unwrap_or_default(),
            icon: device
                .icon()
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| "bluetooth".into()),
            paired: device.is_paired().await.ok().unwrap_or(false),
            trusted: device.is_trusted().await.ok().unwrap_or(false),
            connected: device.is_connected().await.ok().unwrap_or(false),
            battery_percentage: device
                .battery_percentage()
                .await
                .ok()
                .flatten()
                .map(|p| p as i32)
                .unwrap_or(-1),
            rssi: device
                .rssi()
                .await
                .ok()
                .flatten()
                .map(|r| r as i16)
                .unwrap_or(i16::MIN),
        })
    }

    /// Start tracking property changes for a device
    async fn start_tracking_device(
        addr: Address,
        device: &Device,
        device_events: &mut SelectAll<BtDeviceEventStream>,
        tracked_devices: &mut HashSet<Address>,
    ) {
        if tracked_devices.contains(&addr) {
            return;
        }
        match device.events().await {
            Ok(events) => {
                let stream = events.map(move |evt| (addr, evt));
                device_events.push(Box::pin(stream));
                tracked_devices.insert(addr);
            }
            Err(e) => {
                tracing::warn!("Failed to subscribe to events for {}: {}", addr, e);
            }
        }
    }

    /// Rebuild device event streams from current tracked_devices set.
    /// Drops stale streams for devices that were removed during scan.
    pub async fn rebuild_device_streams(
        &self,
        device_events: &mut SelectAll<BtDeviceEventStream>,
        tracked_devices: &mut HashSet<Address>,
    ) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let addrs: Vec<Address> = tracked_devices.drain().collect();
        *device_events = SelectAll::new();
        for addr in addrs {
            if let Ok(device) = adapter.device(addr) {
                Self::start_tracking_device(addr, &device, device_events, tracked_devices).await;
            }
        }
    }

    /// Handle AdapterEvent from discovery stream
    pub async fn handle_adapter_event(
        &self,
        event: AdapterEvent,
        device_events: &mut SelectAll<BtDeviceEventStream>,
        tracked_devices: &mut HashSet<Address>,
    ) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        match event {
            AdapterEvent::DeviceAdded(addr) => {
                if let Ok(device) = adapter.device(addr) {
                    if let Some(data) = Self::read_device_data(&device).await {
                        // Skip devices with no useful name (BLE advertisement noise)
                        if data.name.is_empty() && data.alias == data.address {
                            return;
                        }
                        Self::start_tracking_device(
                            addr,
                            &device,
                            device_events,
                            tracked_devices,
                        )
                        .await;
                        let _ =
                            self.evt_tx.send(BackendEvent::BtDeviceAdded(data)).await;
                    }
                }
            }
            AdapterEvent::DeviceRemoved(addr) => {
                tracked_devices.remove(&addr);
                // BlueZ may fire DeviceRemoved for paired devices during
                // discovery cleanup or BLE timeouts. Re-check with the adapter:
                // if the device still exists and is paired, send an update
                // instead of removing it from the UI.
                if let Some(ref adapter) = self.adapter {
                    if let Ok(device) = adapter.device(addr) {
                        if device.is_paired().await.unwrap_or(false) {
                            if let Some(data) = Self::read_device_data(&device).await {
                                Self::start_tracking_device(
                                    addr,
                                    &device,
                                    device_events,
                                    tracked_devices,
                                ).await;
                                let _ = self.evt_tx.send(BackendEvent::BtDeviceChanged(data)).await;
                                return;
                            }
                        }
                    }
                }
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtDeviceRemoved(addr.to_string()))
                    .await;
            }
            AdapterEvent::PropertyChanged(prop) => match prop {
                AdapterProperty::Discoverable(discoverable) => {
                    let _ = self
                        .evt_tx
                        .send(BackendEvent::BtDiscoverable(discoverable))
                        .await;
                }
                AdapterProperty::Powered(powered) => {
                    let _ = self.evt_tx.send(BackendEvent::BtPowered(powered)).await;
                }
                _ => {}
            },
        }
    }

    /// Handle per-device property change event
    pub async fn handle_device_property_change(
        &self,
        addr: Address,
        property: DeviceProperty,
    ) {
        let dominated = matches!(
            property,
            DeviceProperty::Name(_)
                | DeviceProperty::Alias(_)
                | DeviceProperty::Icon(_)
                | DeviceProperty::Paired(_)
                | DeviceProperty::Trusted(_)
                | DeviceProperty::Connected(_)
                | DeviceProperty::BatteryPercentage(_)
                | DeviceProperty::Rssi(_)
        );
        if !dominated {
            return;
        }

        let Some(ref adapter) = self.adapter else {
            return;
        };

        if let Ok(device) = adapter.device(addr) {
            if let Some(data) = Self::read_device_data(&device).await {
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtDeviceChanged(data))
                    .await;
            }
        }
    }

    /// Start Bluetooth discovery. Returns the stream to be stored externally.
    pub async fn start_scan(&self) -> Option<BtDiscoveryStream> {
        let adapter = self.adapter.as_ref()?;
        match adapter.discover_devices_with_changes().await {
            Ok(stream) => {
                let _ = self.evt_tx.send(BackendEvent::BtDiscovering(true)).await;
                tracing::info!("Bluetooth discovery started");
                Some(Box::pin(stream))
            }
            Err(e) => {
                tracing::error!("Failed to start BT discovery: {}", e);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError(format_bt_error(&e)))
                    .await;
                None
            }
        }
    }

    /// Notify that discovery has been stopped (caller drops the stream)
    pub async fn notify_scan_stopped(&self) {
        tracing::info!("Bluetooth discovery stopped");
        let _ = self
            .evt_tx
            .send(BackendEvent::BtDiscovering(false))
            .await;
    }

    /// Connect to a device by address string
    pub async fn connect(&self, addr_str: &str) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let addr = match Self::parse_address(addr_str) {
            Some(a) => a,
            None => {
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError("Invalid Bluetooth address".into()))
                    .await;
                return;
            }
        };

        let _ = self.evt_tx.send(BackendEvent::BtConnecting(addr_str.to_string())).await;
        match adapter.device(addr) {
            Ok(device) => {
                if let Err(e) = device.connect().await {
                    tracing::error!("BT connect to {} failed: {}", addr, e);
                    let _ = self
                        .evt_tx
                        .send(BackendEvent::BtError(format_bt_error(&e)))
                        .await;
                } else {
                    tracing::info!("Connected to BT device {}", addr);
                }
            }
            Err(e) => {
                tracing::error!("Cannot get device {}: {}", addr, e);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError(format!("Device not found: {}", e)))
                    .await;
            }
        }
    }

    /// Disconnect from a device by address string
    pub async fn disconnect(&self, addr_str: &str) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let Some(addr) = Self::parse_address(addr_str) else {
            return;
        };

        if let Ok(device) = adapter.device(addr) {
            if let Err(e) = device.disconnect().await {
                tracing::error!("BT disconnect from {} failed: {}", addr, e);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError(format_bt_error(&e)))
                    .await;
            }
        }
    }

    /// Pair with a device by address string.
    /// This spawns a separate task because pair() may trigger an agent callback,
    /// and the main select! loop needs to be free to process the BtPairingResponse command.
    pub fn pair(&self, addr_str: &str) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let Some(addr) = Self::parse_address(addr_str) else {
            return;
        };

        let device = match adapter.device(addr) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Cannot get device {} for pairing: {}", addr, e);
                let evt_tx = self.evt_tx.clone();
                tokio::spawn(async move {
                    let _ = evt_tx
                        .send(BackendEvent::BtError(format!("Device not found: {}", e)))
                        .await;
                });
                return;
            }
        };

        let evt_tx = self.evt_tx.clone();
        tokio::spawn(async move {
            let _ = evt_tx.send(BackendEvent::BtConnecting(addr.to_string())).await;
            tracing::info!("Starting pairing with {}", addr);
            if let Err(e) = device.pair().await {
                tracing::error!("BT pair with {} failed: {}", addr, e);
                let _ = evt_tx
                    .send(BackendEvent::BtError(format_bt_error(&e)))
                    .await;
            } else {
                tracing::info!("Paired with BT device {}", addr);
                // Trust the device after pairing so it can auto-connect
                if let Err(e) = device.set_trusted(true).await {
                    tracing::warn!("Failed to set trusted for {}: {}", addr, e);
                }
            }
        });
    }

    /// Remove (unpair) a device by address string
    pub async fn remove(&self, addr_str: &str) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let Some(addr) = Self::parse_address(addr_str) else {
            return;
        };

        match adapter.remove_device(addr).await {
            Ok(()) => {
                tracing::info!("BT device {} removed from BlueZ", addr);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtDeviceRemoved(addr_str.to_string()))
                    .await;
            }
            Err(e) if e.to_string().contains("Does Not Exist") => {
                tracing::info!("BT device {} already gone from BlueZ, removing from UI", addr);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtDeviceRemoved(addr_str.to_string()))
                    .await;
            }
            Err(e) => {
                tracing::error!("BT remove {} failed: {}", addr, e);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError(format_bt_error(&e)))
                    .await;
            }
        }
    }

    /// Set alias (display name) for a device by address string
    pub async fn set_alias(&self, addr_str: &str, alias: &str) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let Some(addr) = Self::parse_address(addr_str) else {
            return;
        };

        if let Ok(device) = adapter.device(addr) {
            if let Err(e) = device.set_alias(alias.to_string()).await {
                tracing::error!("BT set alias for {} failed: {}", addr, e);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError(format_bt_error(&e)))
                    .await;
            }
        }
    }

    /// Set trusted flag for a device by address string
    pub async fn set_trusted_flag(&self, addr_str: &str, trusted: bool) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        let Some(addr) = Self::parse_address(addr_str) else {
            return;
        };

        if let Ok(device) = adapter.device(addr) {
            if let Err(e) = device.set_trusted(trusted).await {
                tracing::error!("BT set trusted {} for {} failed: {}", trusted, addr, e);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtError(format_bt_error(&e)))
                    .await;
            }
        }
    }

    /// Set adapter powered state
    pub async fn set_powered(&self, powered: bool) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        if let Err(e) = adapter.set_powered(powered).await {
            tracing::error!("BT set powered {} failed: {}", powered, e);
            let _ = self
                .evt_tx
                .send(BackendEvent::BtError(format_bt_error(&e)))
                .await;
            // Send actual state back so UI can roll back the optimistic update
            if let Ok(actual) = adapter.is_powered().await {
                let _ = self.evt_tx.send(BackendEvent::BtPowered(actual)).await;
            }
            return;
        }
        let _ = self.evt_tx.send(BackendEvent::BtPowered(powered)).await;
    }

    /// Set adapter discoverable state
    pub async fn set_discoverable(&self, discoverable: bool) {
        let Some(ref adapter) = self.adapter else {
            return;
        };
        if let Err(e) = adapter.set_discoverable(discoverable).await {
            tracing::error!("BT set discoverable {} failed: {}", discoverable, e);
            let _ = self
                .evt_tx
                .send(BackendEvent::BtError(format_bt_error(&e)))
                .await;
            // Send actual state back so UI can roll back the optimistic update
            if let Ok(actual) = adapter.is_discoverable().await {
                let _ = self
                    .evt_tx
                    .send(BackendEvent::BtDiscoverable(actual))
                    .await;
            }
            return;
        }
        let _ = self
            .evt_tx
            .send(BackendEvent::BtDiscoverable(discoverable))
            .await;
    }

    fn parse_address(addr_str: &str) -> Option<Address> {
        match addr_str.parse() {
            Ok(a) => Some(a),
            Err(e) => {
                tracing::error!("Invalid BT address '{}': {}", addr_str, e);
                None
            }
        }
    }
}
