# Xandeum Dock Setup

Xandeum Dock connects the local RPC node to Atlas. It reads and writes Unix
socket files under `/var/run/xandeum`, so that runtime directory must exist and
be owned by the RPC service user before Dock starts.

## Clone The Repository

```bash
git clone git@github.com:Xandeum/dock.git
cd dock
```

If you need HTTPS instead of SSH:

```bash
git clone https://github.com/Xandeum/dock.git
cd dock
```

## Install Build Dependencies

On Ubuntu or Debian:

```bash
sudo apt-get update
sudo apt-get install -y build-essential git pkg-config libzmq3-dev protobuf-compiler
```

## Build Dock

```bash
cargo build --release
```

The release binary is created at:

```bash
target/release/xandeum-dock
```

## Create The Runtime Socket Directory

Dock uses these socket paths:

```text
/var/run/xandeum/fromdock.sock
/var/run/xandeum/todock.sock
```

Create the directory and give ownership to the RPC service user so the RPC node
can create the socket files. The included service files use `User=sol`; if your
RPC runs as a different user, replace `sol:sol` with that user and group.

```bash
sudo install -d -o sol -g sol -m 0755 /var/run/xandeum
sudo chown -R sol:sol /var/run/xandeum
```

Start the RPC node first so it can create the socket files, then start Dock:

```bash
sudo systemctl start <rpc-service>.service
sudo ls -la /var/run/xandeum
```

Replace `<rpc-service>.service` with the actual RPC or validator service name.

## Choose The Atlas Cluster

Update the `ExecStart` line in `vega.service` and `altair.service` for the
cluster you are running.

Use Devnet Atlas:

```ini
ExecStart=/usr/bin/xandeum-dock --version vega --tcp-push atlas.devnet.xandeum.com:3001 --tcp-pull atlas.devnet.xandeum.com:4001
ExecStart=/usr/bin/xandeum-dock --version altair --tcp-push atlas.devnet.xandeum.com:3001 --tcp-pull atlas.devnet.xandeum.com:4001
```

Use Trynet Atlas:

```ini
ExecStart=/usr/bin/xandeum-dock --version vega --tcp-push atlas.trynet.xandeum.com:3001 --tcp-pull atlas.trynet.xandeum.com:4001
ExecStart=/usr/bin/xandeum-dock --version altair --tcp-push atlas.trynet.xandeum.com:3001 --tcp-pull atlas.trynet.xandeum.com:4001
```

Use Mainnet Atlas:

```ini
ExecStart=/usr/bin/xandeum-dock --version vega --tcp-push atlas.mainnet.xandeum.com:3001 --tcp-pull atlas.mainnet.xandeum.com:4001
ExecStart=/usr/bin/xandeum-dock --version altair --tcp-push atlas.mainnet.xandeum.com:3001 --tcp-pull atlas.mainnet.xandeum.com:4001
```

Use `atlas.devnet.xandeum.com`, `atlas.trynet.xandeum.com`, or
`atlas.mainnet.xandeum.com` depending on your cluster. After editing the service
files, run the update commands below.

## Update, Build, And Install

This script stops the Dock services, pulls the latest Dock changes, updates the
latest protos submodule, builds the release binary, installs it to `/usr/bin`,
copies the systemd service files, reloads systemd, and restarts Dock.

Run this from inside the cloned `dock` repository. This repo currently uses
`master`; if your checkout uses `main`, run the script with
`DOCK_BRANCH=main PROTO_BRANCH=main`.

```bash
#!/bin/bash
set -e

sudo systemctl stop vega.service || true
sudo systemctl stop altair.service || true

DOCK_BRANCH="${DOCK_BRANCH:-master}"
PROTO_BRANCH="${PROTO_BRANCH:-master}"

# Pull latest Dock changes from master or main.
git fetch origin
git switch "$DOCK_BRANCH" 2>/dev/null || git switch -c "$DOCK_BRANCH" --track "origin/$DOCK_BRANCH"
git pull --ff-only origin "$DOCK_BRANCH"

# Pull latest proto changes from master or main.
git submodule sync --recursive
git submodule update --init --recursive
git -C xandeum-protos fetch origin
git -C xandeum-protos switch "$PROTO_BRANCH" 2>/dev/null || git -C xandeum-protos switch -c "$PROTO_BRANCH" --track "origin/$PROTO_BRANCH"
git -C xandeum-protos pull --ff-only origin "$PROTO_BRANCH"

# Build release binary.
cargo build --release

# Install binary.
sudo cp target/release/xandeum-dock /usr/bin/xandeum-dock
sudo chmod +x /usr/bin/xandeum-dock

# Make sure the runtime socket directory exists.
sudo install -d -o sol -g sol -m 0755 /var/run/xandeum
sudo chown -R sol:sol /var/run/xandeum

# Install systemd services.
sudo cp vega.service /etc/systemd/system/vega.service
sudo cp altair.service /etc/systemd/system/altair.service
sudo systemctl daemon-reload

# Start or restart Dock.
sudo systemctl enable vega.service
sudo systemctl enable altair.service
sudo systemctl restart vega.service
sudo systemctl restart altair.service
```

## Verify Services

```bash
sudo systemctl status vega.service
sudo systemctl status altair.service

journalctl -u vega.service -f
journalctl -u altair.service -f
```
