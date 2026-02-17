# wlcontrol

WiFi and Bluetooth control app for Linux, built with GTK4/libadwaita.

Uses [IWD](https://iwd.wiki.kernel.org/) for WiFi and [BlueZ](http://www.bluez.org/) for Bluetooth.

## Build Dependencies

### Ubuntu / Debian

```bash
sudo apt install pkg-config libdbus-1-dev libgdk-pixbuf-2.0-dev libgtk-4-dev libadwaita-1-dev blueprint-compiler
```

## Build

```bash
cargo build
```

Blueprint files (`.blp`) are compiled to GTK UI files automatically during `cargo build`.

## Run

```bash
cargo run
```

Enable debug logging:

```bash
RUST_LOG=debug cargo run
```
