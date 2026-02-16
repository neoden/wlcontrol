# wlcontrol - WiFi/Bluetooth Control App

## Overview

Rust + libadwaita app for managing WiFi (iwd) and Bluetooth (BlueZ) via D-Bus.
UI follows pwvucontrol patterns: AdwViewSwitcher in header, AdwPreferencesGroup with rows.

## Tech Stack

- **UI**: gtk4-rs 0.10 + libadwaita 0.8 (GTK 4.16 / libadwaita 1.6)
- **D-Bus**: zbus 4.4 (pure Rust, async)
- **Bluetooth**: bluer 0.17 (official BlueZ bindings)
- **WiFi**: Custom zbus proxies for iwd (no existing crate)
- **Async**: tokio runtime, async-channel for backend↔UI communication
- **Build**: cargo (meson integration готов, но для разработки достаточно cargo)

## Project Structure

```
wlcontrol/
├── Cargo.toml
├── meson.build
├── build.rs
├── data/
│   ├── meson.build
│   ├── resources/
│   │   ├── meson.build
│   │   ├── resources.gresource.xml
│   │   ├── style.css               # Custom CSS (spin, pulse animations)
│   │   └── ui/
│   │       ├── window.blp + .ui
│   │       ├── wifi-page.blp + .ui
│   │       ├── bluetooth-page.blp + .ui
│   │       ├── wifi-network-row.blp + .ui
│   │       ├── bluetooth-device-row.blp + .ui
│   │       └── password-dialog.blp + .ui
│   └── dev.neoden.wlcontrol.desktop.in
└── src/
    ├── main.rs
    ├── application.rs
    ├── backend/
    │   ├── mod.rs
    │   ├── manager.rs              # Central coordinator + BackendCommand/BackendEvent
    │   ├── wifi/
    │   │   ├── mod.rs
    │   │   ├── iwd_agent.rs        # D-Bus Agent for password prompts
    │   │   ├── iwd_proxy.rs        # zbus proxy traits (Station, Network, KnownNetwork, Device)
    │   │   └── network.rs          # GObject: WifiNetwork
    │   └── bluetooth/
    │       ├── mod.rs
    │       └── device.rs           # GObject: BtDevice
    └── ui/
        ├── mod.rs
        ├── window.rs
        ├── wifi_page.rs
        ├── bluetooth_page.rs
        ├── wifi_network_row.rs
        ├── bluetooth_device_row.rs
        └── password_dialog.rs
```

## Architecture

```
D-Bus (iwd/BlueZ)
       │
       ▼ PropertyChanged signals
┌──────────────────────────────┐
│  Backend (tokio thread)      │
│  - zbus proxies (iwd)        │
│  - bluer Session (BlueZ)     │
│  - async-channel (commands)  │
└──────────┬───────────────────┘
           │ BackendEvent → glib::spawn_future_local()
           ▼
┌──────────────────────────────┐
│  GObject Models (GTK thread) │
│  - ListStore<WifiNetwork>    │
│  - ListStore<BtDevice>       │
│  - FilterListModel (paired/  │
│    connected/discovered)     │
└──────────┬───────────────────┘
           │ bind_property() / bind_model()
           ▼
┌──────────────────────────────┐
│  UI Widgets                  │
│  - AdwActionRow rows         │
│  - Automatic sync via        │
│    property bindings         │
└──────────────────────────────┘
```

## Key Patterns

1. **GObject Subclassing** for reactive data models
2. **async-channel** for bidirectional UI↔backend communication
3. **Composite templates** (`.blp` files compiled via blueprint-compiler)
4. **FilterListModel** for categorizing BT devices (connected/paired/discovered)
5. **Property bindings** (.sync_create().bidirectional().build())
6. **Error handling** — Result propagation, user-facing errors via AdwToast

## iwd D-Bus API

Service: `net.connman.iwd`

| Interface | Key Methods/Properties |
|-----------|----------------------|
| Station | `Scan()`, `Disconnect()`, `GetOrderedNetworks()`, `state`, `scanning`, `connected_network` |
| Network | `Connect()`, `name`, `type`, `connected` |
| KnownNetwork | `Forget()`, `name`, `auto_connect`, `last_connected_time` |
| Device | `powered`, `address`, `mode` |
| Adapter | `name`, `model`, `vendor`, `supported_modes` |
| AgentManager | `RegisterAgent()`, `UnregisterAgent()` |
| Agent (impl) | `RequestPassphrase()`, `Cancel()`, `Release()` |

**ObjectManager сигналы (на пути `/`):**
- `InterfacesAdded(path, interfaces)` — новый объект появился (адаптер подключён, WiFi включён)
- `InterfacesRemoved(path, interfaces)` — объект удалён (адаптер отключён)

**org.freedesktop.DBus:**
- `NameOwnerChanged(name, old_owner, new_owner)` — сервис появился/исчез (iwd запущен/остановлен)

## BlueZ API (via bluer crate)

```rust
let session = bluer::Session::new().await?;
let adapter = session.default_adapter().await?;
adapter.set_powered(true).await?;
adapter.discover_devices().await?;  // Stream of events
device.pair().await?;
device.connect().await?;
```

## Implementation Phases

### Phase 1: Project Setup ✅ DONE
- [x] Create meson.build + Cargo.toml
- [x] Blueprint compiler integration
- [x] Basic main.rs with AdwApplication
- [x] Empty module structure
- [x] Window with AdwViewStack skeleton
- [x] Backend manager with async-channel
- [x] GObjects: WifiNetwork, BtDevice
- [x] UI pages with property bindings

### Phase 2: iwd Integration ✅ DONE
- [x] Connect to iwd D-Bus service in run_backend()
- [x] Find Station interface via ObjectManager (on "/" path)
- [x] Watch PropertyChanged signals (Device.powered, Station.scanning, Station.state)
- [x] Implement WifiScan command → Station.Scan()
- [x] Implement GetOrderedNetworks → populate wifi_networks ListStore
- [x] Send network list updates to UI via BackendEvent
- [x] Implement WifiConnect command → Network.Connect()
- [x] Implement WifiDisconnect command
- [x] Implement WifiSetPowered → Device.powered

Note: Signal strength from iwd is in cBm (centiBels), divided by 100 for dBm display.

### Phase 3: iwd Agent (Password Prompts) ✅ DONE
- [x] Implement net.connman.iwd.Agent interface via zbus
- [x] Register agent with AgentManager
- [x] Handle RequestPassphrase method → send event to UI
- [x] PasswordDialog integration
- [x] Return passphrase to iwd

### Phase 4: Known Networks ✅ DONE
- [x] Known networks shown in main list with "Saved" subtitle
- [x] Forget button for saved networks
- [x] WifiForget command → KnownNetwork.Forget()
- [x] Dynamic known status update after connect
- [ ] Auto-connect toggle (optional, low priority)
- [ ] Captive portal detection (check connectivity after connect, open browser if redirect)

### Phase 4.5: D-Bus Robustness (Hot-plug & Service Monitoring)

Неучтённые D-Bus события, необходимые для надёжной работы:

**P0 — Критично (влияет на работоспособность):**
- [ ] `ObjectManager.InterfacesAdded` — обнаружение появления WiFi адаптера (USB донгл подключён)
- [ ] `ObjectManager.InterfacesRemoved` — обнаружение удаления адаптера (донгл отключён)
- [ ] `org.freedesktop.DBus.NameOwnerChanged` для `net.connman.iwd` — отслеживание запуска/остановки iwd

Сейчас: если iwd не запущен при старте → приложение не работает; если адаптер подключить/отключить → не обновится.

**P1 — Желательно (улучшение UX):**
- [ ] `Station.connected_network` PropertyChanged — точное отслеживание текущей сети (сейчас только `state`)
- [ ] `Device.mode` PropertyChanged — обнаружение смены режима (station → ap)
- [ ] `KnownNetwork.auto_connect` PropertyChanged — отслеживание изменений из других приложений

**P2 — Опционально (дополнительные фичи):**
- [ ] `net.connman.iwd.SignalLevelAgent` — уведомления об изменении уровня сигнала без повторного сканирования
- [ ] `Adapter.powered` — rfkill состояние адаптера (hard block)

Реализация P0:
```rust
// В run_backend():
let obj_manager = ObjectManagerProxy::new(&conn).await?;
let mut interfaces_added = obj_manager.receive_interfaces_added().await?;
let mut interfaces_removed = obj_manager.receive_interfaces_removed().await?;

// В select! loop:
Some(signal) = interfaces_added.next() => {
    // Проверить, появился ли Device/Station интерфейс
    // Переинициализировать wifi backend
}
Some(signal) = interfaces_removed.next() => {
    // Проверить, удалён ли наш Device
    // Очистить состояние, показать "No WiFi adapter"
}
```

### Phase 5: BlueZ Integration
- [ ] Initialize bluer Session in run_backend()
- [ ] Get default adapter, watch powered state
- [ ] Device discovery stream → populate bt_devices ListStore
- [ ] Implement BtPair, BtConnect, BtDisconnect, BtRemove commands
- [ ] Battery level reading (org.bluez.Battery1)
- [ ] Discoverable toggle

### Phase 6: UI Polish
- [x] AdwToast notifications for errors
- [x] Loading states (pulsing indicator on network row)
- [x] Scan button animation (rotating refresh icon)
- [x] Click connected network to disconnect
- [ ] Signal strength icons update (live)
- [ ] Device type icons for Bluetooth
- [ ] Desktop file + icons

## Build & Run

```bash
# Compile blueprints (one-time or after .blp changes)
blueprint-compiler batch-compile data/resources/ui data/resources/ui data/resources/ui/*.blp

# Build and run
cargo build && ./target/debug/wlcontrol

# With debug logging
RUST_LOG=debug cargo run
```

## Current State

Приложение полностью функционально для WiFi:
- WiFi on/off переключатель
- Сканирование сетей с анимацией вращения иконки refresh
- Подключение к known networks (один клик)
- Подключение к новым сетям с паролем (Agent + PasswordDialog)
- Отключение по клику на подключённую сеть
- Индикатор подключения (пульсирующий круг)
- Known networks: subtitle "Saved" + кнопка Forget
- Toast уведомления об ошибках
- Автоматическое сканирование при включении WiFi
- Сохранение галочки подключения при отмене ввода пароля

Bluetooth — stub (Phase 5).

**Известные ограничения:**
- Нет поддержки hot-plug WiFi адаптеров (InterfacesAdded/Removed)
- Нет отслеживания запуска/остановки iwd (NameOwnerChanged)
- Требуется запущенный iwd при старте приложения

Следующий шаг: Phase 4.5 (D-Bus Robustness) или Phase 5 (BlueZ Integration).

## Files Reference

Key files:
- `src/backend/manager.rs` — run_backend(), command/event handling, iwd integration
- `src/backend/wifi/iwd_agent.rs` — D-Bus Agent for password prompts (#[interface] impl)
- `src/backend/wifi/iwd_proxy.rs` — zbus proxy traits (Station, Network, Device, AgentManager)
- `src/backend/wifi/network.rs` — WifiNetwork GObject (path, name, connected, connecting, etc.)
- `src/ui/wifi_page.rs` — WiFi UI page, signal handlers, PasswordDialog integration
- `data/resources/style.css` — CSS animations (spinning refresh icon)

BackendCommand enum (manager.rs):
- WifiScan, WifiConnect{path}, WifiDisconnect, WifiForget{path}, WifiSetPowered(bool)
- PassphraseResponse{passphrase: Option<String>}
- BtScan, BtStopScan, BtConnect{path}, BtDisconnect{path}, BtPair{path}, BtRemove{path}
- BtSetPowered(bool), BtSetDiscoverable(bool)

BackendEvent enum (manager.rs):
- WifiPowered(bool), WifiScanning(bool), WifiNetworks(Vec), WifiConnected(Option), WifiConnecting(path), WifiNetworkKnown{path}
- PassphraseRequest{network_path, network_name}
- BtPowered(bool), BtDiscovering(bool), BtDiscoverable(bool)
- Error(String)
