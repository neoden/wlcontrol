use std::collections::HashSet;

use async_channel::Sender;
use tokio::sync::oneshot;
use zbus::zvariant::OwnedObjectPath;

use super::super::bluetooth::backend::{BluetoothBackend, BtPairingRequest};
use super::super::manager::{BackendCommand, BackendEvent, BtPairingKind};
use super::super::wifi_backend::{find_all_iwd_devices, IwdDeviceInfo, WifiBackend};
use super::helpers::{
    create_device_proxy, send_wifi_initial_state, setup_station_streams,
    setup_station_streams_with_retry,
};
use super::streams::EventStreams;
use super::LoopEvent;

pub enum LoopAction {
    Continue,
    Break,
}

pub struct BackendState {
    pub conn: zbus::Connection,
    pub evt_tx: Sender<BackendEvent>,
    pub wifi: Option<WifiBackend>,
    pub bt: Option<BluetoothBackend>,
    pub bt_tracked_devices: HashSet<bluer::Address>,
    pub wifi_device_infos: Vec<IwdDeviceInfo>,
    pub pending_passphrase_response: Option<oneshot::Sender<Option<String>>>,
    pub pending_pairing_response: Option<oneshot::Sender<Result<(), bluer::agent::ReqError>>>,
    pub pending_pin_response: Option<oneshot::Sender<Result<String, bluer::agent::ReqError>>>,
    pub pending_passkey_response: Option<oneshot::Sender<Result<u32, bluer::agent::ReqError>>>,
}

impl BackendState {
    pub async fn handle_event(
        &mut self,
        event: LoopEvent,
        streams: &mut EventStreams,
    ) -> LoopAction {
        match event {
            LoopEvent::BtScanTimeout => {
                tracing::info!("Bluetooth discovery timeout (30s), stopping scan");
                if streams.bt_discovery_stream.take().is_some() {
                    if let Some(ref bt_backend) = self.bt {
                        bt_backend.notify_scan_stopped().await;
                    }
                }
                streams.bt_scan_deadline = None;
            }

            LoopEvent::WifiPoweredChanged(powered) => {
                tracing::info!("Device powered changed: {}", powered);
                let _ = self.evt_tx.send(BackendEvent::WifiPowered(powered)).await;

                if let Some(ref w) = self.wifi {
                    if let Some(path) = w.device_path() {
                        if powered {
                            let (scanning, state) =
                                setup_station_streams_with_retry(&self.conn, path).await;
                            streams.station_scanning_stream = scanning;
                            streams.station_state_stream = state;
                            w.send_networks().await;
                            w.send_known_networks().await;
                        } else {
                            streams.station_scanning_stream = None;
                            streams.station_state_stream = None;
                            let _ = self
                                .evt_tx
                                .send(BackendEvent::WifiNetworks(vec![]))
                                .await;
                        }
                    }
                }
            }

            LoopEvent::WifiScanningChanged(scanning) => {
                tracing::debug!("Station scanning changed: {}", scanning);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::WifiScanning(scanning))
                    .await;
                if !scanning {
                    if let Some(ref w) = self.wifi {
                        w.send_networks().await;
                        w.send_known_networks().await;
                    }
                }
            }

            LoopEvent::WifiStationStateChanged(state) => {
                tracing::info!("Station state changed: {}", state);
                if let Some(ref w) = self.wifi {
                    w.send_connected_status().await;
                }
            }

            LoopEvent::PassphraseRequest(request) => {
                tracing::info!(
                    "Passphrase request: {} ({})",
                    request.network_name,
                    request.network_path
                );
                self.pending_passphrase_response = Some(request.response_tx);
                let _ = self
                    .evt_tx
                    .send(BackendEvent::PassphraseRequest {
                        network_path: request.network_path,
                        network_name: request.network_name,
                    })
                    .await;
            }

            LoopEvent::BtDiscoveryEvent(adapter_event) => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend
                        .handle_adapter_event(
                            adapter_event,
                            &mut streams.bt_device_events,
                            &mut self.bt_tracked_devices,
                        )
                        .await;
                }
            }

            LoopEvent::BtAdapterEvent(adapter_event) => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend
                        .handle_adapter_event(
                            adapter_event,
                            &mut streams.bt_device_events,
                            &mut self.bt_tracked_devices,
                        )
                        .await;
                }
            }

            LoopEvent::BtDevicePropertyChanged { address, property } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend
                        .handle_device_property_change(address, property)
                        .await;
                }
            }

            LoopEvent::BtPairingRequest(request) => {
                self.handle_bt_pairing_request(request).await;
            }

            LoopEvent::IwdDeviceAdded { object_path } => {
                self.handle_iwd_device_added(&object_path).await;
            }

            LoopEvent::IwdDeviceRemoved { object_path } => {
                self.handle_iwd_device_removed(&object_path, streams).await;
            }

            LoopEvent::Command(cmd) => {
                return self.handle_command(cmd, streams).await;
            }

            LoopEvent::CommandChannelClosed => {
                return LoopAction::Break;
            }
        }

        LoopAction::Continue
    }

    async fn handle_command(
        &mut self,
        cmd: BackendCommand,
        streams: &mut EventStreams,
    ) -> LoopAction {
        tracing::debug!("Received command: {:?}", cmd);
        match cmd {
            BackendCommand::Shutdown => {
                tracing::info!("Backend shutdown requested");
                if let Some(ref w) = self.wifi {
                    w.shutdown();
                }
                streams.bt_discovery_stream.take();
                return LoopAction::Break;
            }
            BackendCommand::PassphraseResponse { passphrase } => {
                if let Some(tx) = self.pending_passphrase_response.take() {
                    let _ = tx.send(passphrase);
                }
            }
            BackendCommand::WifiScan => {
                if let Some(ref w) = self.wifi {
                    w.scan().await;
                }
            }
            BackendCommand::WifiConnect { path } => {
                if let Some(ref w) = self.wifi {
                    w.connect(&path).await;
                }
            }
            BackendCommand::WifiDisconnect => {
                if let Some(ref w) = self.wifi {
                    w.disconnect().await;
                }
            }
            BackendCommand::WifiForget { path } => {
                if let Some(ref w) = self.wifi {
                    w.forget(&path).await;
                }
            }
            BackendCommand::WifiForgetKnown { path } => {
                if let Some(ref w) = self.wifi {
                    w.forget_known(&path).await;
                }
            }
            BackendCommand::WifiSetPowered { powered } => {
                if let Some(ref w) = self.wifi {
                    w.set_powered(powered).await;
                }
            }
            BackendCommand::WifiSwitchAdapter { device_path } => {
                self.handle_wifi_switch_adapter(&device_path, streams)
                    .await;
            }
            BackendCommand::BtScan => {
                if streams.bt_discovery_stream.is_none() {
                    if let Some(ref bt_backend) = self.bt {
                        streams.bt_discovery_stream = bt_backend.start_scan().await;
                        if streams.bt_discovery_stream.is_some() {
                            streams.bt_scan_deadline = Some(
                                tokio::time::Instant::now()
                                    + std::time::Duration::from_secs(30),
                            );
                        }
                    }
                }
            }
            BackendCommand::BtStopScan => {
                if streams.bt_discovery_stream.take().is_some() {
                    streams.bt_scan_deadline = None;
                    if let Some(ref bt_backend) = self.bt {
                        bt_backend.notify_scan_stopped().await;
                        bt_backend
                            .rebuild_device_streams(
                                &mut streams.bt_device_events,
                                &mut self.bt_tracked_devices,
                            )
                            .await;
                    }
                }
            }
            BackendCommand::BtConnect { path } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.connect(&path).await;
                }
            }
            BackendCommand::BtDisconnect { path } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.disconnect(&path).await;
                }
            }
            BackendCommand::BtPair { path } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.pair(&path);
                }
            }
            BackendCommand::BtRemove { path } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.remove(&path).await;
                }
            }
            BackendCommand::BtSetAlias { path, alias } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.set_alias(&path, &alias).await;
                }
            }
            BackendCommand::BtSetTrusted { path, trusted } => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.set_trusted_flag(&path, trusted).await;
                }
            }
            BackendCommand::BtSetPowered(powered) => {
                if !powered {
                    if streams.bt_discovery_stream.take().is_some() {
                        if let Some(ref bt_backend) = self.bt {
                            bt_backend.notify_scan_stopped().await;
                        }
                    }
                    streams.bt_scan_deadline = None;
                    self.bt_tracked_devices.clear();
                    streams.bt_device_events = futures::stream::SelectAll::new();
                    streams.bt_adapter_events = None;
                }
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.set_powered(powered).await;
                    if powered {
                        bt_backend
                            .send_initial_state(
                                &mut streams.bt_device_events,
                                &mut self.bt_tracked_devices,
                            )
                            .await;
                        streams.bt_adapter_events = bt_backend.adapter_events().await;
                    }
                }
            }
            BackendCommand::BtSetDiscoverable(discoverable) => {
                if let Some(ref bt_backend) = self.bt {
                    bt_backend.set_discoverable(discoverable).await;
                }
            }
            BackendCommand::BtPairingResponse { accept } => {
                if let Some(tx) = self.pending_pairing_response.take() {
                    let result = if accept {
                        Ok(())
                    } else {
                        Err(bluer::agent::ReqError::Rejected)
                    };
                    let _ = tx.send(result);
                }
            }
            BackendCommand::BtPairingPinResponse { pin } => {
                if let Some(tx) = self.pending_pin_response.take() {
                    let result = match pin {
                        Some(p) => Ok(p),
                        None => Err(bluer::agent::ReqError::Rejected),
                    };
                    let _ = tx.send(result);
                }
            }
            BackendCommand::BtPairingPasskeyResponse { passkey } => {
                if let Some(tx) = self.pending_passkey_response.take() {
                    let result = match passkey {
                        Some(k) => Ok(k),
                        None => Err(bluer::agent::ReqError::Rejected),
                    };
                    let _ = tx.send(result);
                }
            }
        }
        LoopAction::Continue
    }

    async fn handle_bt_pairing_request(&mut self, request: BtPairingRequest) {
        let (kind, address) = match request {
            BtPairingRequest::ConfirmPasskey {
                address,
                passkey,
                response_tx,
            } => {
                self.pending_pairing_response = Some(response_tx);
                (BtPairingKind::ConfirmPasskey(format!("{:06}", passkey)), address)
            }
            BtPairingRequest::RequestPinCode {
                address,
                response_tx,
            } => {
                self.pending_pin_response = Some(response_tx);
                (BtPairingKind::RequestPin, address)
            }
            BtPairingRequest::RequestPasskey {
                address,
                response_tx,
            } => {
                self.pending_passkey_response = Some(response_tx);
                (BtPairingKind::RequestPasskey, address)
            }
            BtPairingRequest::DisplayPasskey { address, passkey } => {
                (BtPairingKind::DisplayPasskey(format!("{:06}", passkey)), address)
            }
            BtPairingRequest::DisplayPinCode { address, pin_code } => {
                (BtPairingKind::DisplayPin(pin_code), address)
            }
            BtPairingRequest::RequestAuthorization {
                address,
                response_tx,
            } => {
                self.pending_pairing_response = Some(response_tx);
                (BtPairingKind::Authorize, address)
            }
        };
        tracing::info!("BT pairing {:?} for {}", kind, address);
        let _ = self
            .evt_tx
            .send(BackendEvent::BtPairing {
                kind,
                address: address.to_string(),
            })
            .await;
    }

    async fn handle_iwd_device_added(&mut self, object_path: &str) {
        tracing::info!("iwd device added: {}", object_path);
        if let Ok(infos) = find_all_iwd_devices(&self.conn).await {
            self.wifi_device_infos = infos;
            let active = self
                .wifi
                .as_ref()
                .and_then(|w| w.device_path())
                .map(|p| p.to_string());
            let _ = self
                .evt_tx
                .send(BackendEvent::WifiDevices {
                    devices: self.wifi_device_infos.clone(),
                    active_path: active,
                })
                .await;
        }
    }

    async fn handle_iwd_device_removed(
        &mut self,
        removed_path: &str,
        streams: &mut EventStreams,
    ) {
        tracing::info!("iwd device removed: {}", removed_path);

        let active_removed = self
            .wifi
            .as_ref()
            .and_then(|w| w.device_path())
            .map(|p| p.as_str() == removed_path)
            .unwrap_or(false);

        if let Ok(infos) = find_all_iwd_devices(&self.conn).await {
            self.wifi_device_infos = infos;
        }

        if active_removed {
            if let Some(info) = self.wifi_device_infos.first() {
                let path: OwnedObjectPath = info.device_path.as_str().try_into().unwrap();
                self.wifi =
                    Some(WifiBackend::new(self.conn.clone(), self.evt_tx.clone(), path.clone()));
                streams.device_powered_stream =
                    if let Some(device) = create_device_proxy(&self.conn, &path).await {
                        Some(device.receive_powered_changed().await)
                    } else {
                        None
                    };
                let (scanning, state) = setup_station_streams(&self.conn, &path).await;
                streams.station_scanning_stream = scanning;
                streams.station_state_stream = state;
                send_wifi_initial_state(&self.conn, &path, &self.evt_tx).await;
            } else {
                self.wifi = None;
                streams.device_powered_stream = None;
                streams.station_scanning_stream = None;
                streams.station_state_stream = None;
                let _ = self
                    .evt_tx
                    .send(BackendEvent::WifiPowered(false))
                    .await;
                let _ = self
                    .evt_tx
                    .send(BackendEvent::WifiNetworks(vec![]))
                    .await;
            }
        }

        let active = self
            .wifi
            .as_ref()
            .and_then(|w| w.device_path())
            .map(|p| p.to_string());
        let _ = self
            .evt_tx
            .send(BackendEvent::WifiDevices {
                devices: self.wifi_device_infos.clone(),
                active_path: active,
            })
            .await;
    }

    async fn handle_wifi_switch_adapter(
        &mut self,
        device_path: &str,
        streams: &mut EventStreams,
    ) {
        tracing::info!("Switching WiFi adapter to {}", device_path);
        if let Some(ref w) = self.wifi {
            w.shutdown();
        }
        let path: OwnedObjectPath = device_path.try_into().unwrap();
        self.wifi = Some(WifiBackend::new(
            self.conn.clone(),
            self.evt_tx.clone(),
            path.clone(),
        ));
        streams.device_powered_stream =
            if let Some(device) = create_device_proxy(&self.conn, &path).await {
                Some(device.receive_powered_changed().await)
            } else {
                None
            };
        let (scanning, state) = setup_station_streams(&self.conn, &path).await;
        streams.station_scanning_stream = scanning;
        streams.station_state_stream = state;
        send_wifi_initial_state(&self.conn, &path, &self.evt_tx).await;
    }
}
