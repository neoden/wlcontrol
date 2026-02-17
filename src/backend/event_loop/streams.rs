use async_channel::Receiver;
use futures::stream::SelectAll;
use futures::StreamExt;

use super::super::bluetooth::backend::{
    BtAdapterEventStream, BtDeviceEventStream, BtDiscoveryStream, BtPairingRequest,
};
use super::super::manager::BackendCommand;
use super::super::wifi::PassphraseRequest;
use super::helpers::{next_iwd_added, next_iwd_removed};
use super::LoopEvent;

pub struct EventStreams {
    pub cmd_rx: Receiver<BackendCommand>,
    pub passphrase_rx: Receiver<PassphraseRequest>,
    pub bt_pairing_rx: Option<Receiver<BtPairingRequest>>,

    pub device_powered_stream: Option<zbus::PropertyStream<'static, bool>>,
    pub station_scanning_stream: Option<zbus::PropertyStream<'static, bool>>,
    pub station_state_stream: Option<zbus::PropertyStream<'static, String>>,

    pub bt_discovery_stream: Option<BtDiscoveryStream>,
    pub bt_adapter_events: Option<BtAdapterEventStream>,
    pub bt_device_events: SelectAll<BtDeviceEventStream>,

    pub bt_scan_deadline: Option<tokio::time::Instant>,

    pub iwd_interfaces_added: Option<zbus::fdo::InterfacesAddedStream<'static>>,
    pub iwd_interfaces_removed: Option<zbus::fdo::InterfacesRemovedStream<'static>>,
}

impl EventStreams {
    pub async fn next_event(&mut self) -> LoopEvent {
        loop {
            tokio::select! {
                // BT scan timeout
                _ = async {
                    match self.bt_scan_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                } => {
                    return LoopEvent::BtScanTimeout;
                }

                // Device.Powered property change
                Some(change) = async {
                    match self.device_powered_stream.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match change.get().await {
                        Ok(powered) => return LoopEvent::WifiPoweredChanged(powered),
                        Err(e) => {
                            tracing::warn!("Failed to get Device.Powered: {}", e);
                            continue;
                        }
                    }
                }

                // Station.Scanning property change
                Some(change) = async {
                    match self.station_scanning_stream.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match change.get().await {
                        Ok(scanning) => return LoopEvent::WifiScanningChanged(scanning),
                        Err(e) => {
                            tracing::warn!("Failed to get Station.Scanning: {}", e);
                            continue;
                        }
                    }
                }

                // Station.State property change
                Some(change) = async {
                    match self.station_state_stream.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match change.get().await {
                        Ok(state) => return LoopEvent::WifiStationStateChanged(state),
                        Err(e) => {
                            tracing::warn!("Failed to get Station.State: {}", e);
                            continue;
                        }
                    }
                }

                // Passphrase requests from iwd agent
                Ok(request) = self.passphrase_rx.recv() => {
                    return LoopEvent::PassphraseRequest(request);
                }

                // BT discovery stream events
                Some(event) = async {
                    match self.bt_discovery_stream.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    return LoopEvent::BtDiscoveryEvent(event);
                }

                // BT always-on adapter events
                Some(event) = async {
                    match self.bt_adapter_events.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    return LoopEvent::BtAdapterEvent(event);
                }

                // BT per-device property changes
                Some((addr, event)) = self.bt_device_events.next() => {
                    let bluer::DeviceEvent::PropertyChanged(property) = event;
                    return LoopEvent::BtDevicePropertyChanged { address: addr, property };
                }

                // BT pairing agent requests
                Ok(request) = async {
                    match self.bt_pairing_rx.as_ref() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    return LoopEvent::BtPairingRequest(request);
                }

                // iwd InterfacesAdded (hot-plug)
                Some(signal) = next_iwd_added(&mut self.iwd_interfaces_added) => {
                    match signal.args() {
                        Ok(args) => {
                            if args.interfaces_and_properties().contains_key("net.connman.iwd.Device") {
                                return LoopEvent::IwdDeviceAdded {
                                    object_path: args.object_path().to_string(),
                                };
                            }
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse InterfacesAdded: {}", e);
                            continue;
                        }
                    }
                }

                // iwd InterfacesRemoved (hot-plug)
                Some(signal) = next_iwd_removed(&mut self.iwd_interfaces_removed) => {
                    match signal.args() {
                        Ok(args) => {
                            if args.interfaces().contains(&"net.connman.iwd.Device") {
                                return LoopEvent::IwdDeviceRemoved {
                                    object_path: args.object_path().to_string(),
                                };
                            }
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse InterfacesRemoved: {}", e);
                            continue;
                        }
                    }
                }

                // UI commands
                result = self.cmd_rx.recv() => {
                    match result {
                        Ok(cmd) => return LoopEvent::Command(cmd),
                        Err(_) => return LoopEvent::CommandChannelClosed,
                    }
                }
            }
        }
    }
}
