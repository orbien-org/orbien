---
sidebar_position: 1
sidebar_label: systemd
title: systemd
---

## Linux

This guide walks through setting up **orbien-server** as a systemd service on Linux.

1. Install systemd

```shell
# CentOS/RHEL
yum install systemd

# Debian/Ubuntu
apt install systemd
```
2. Create the orbien-server service unit

```toml
sudo tee /etc/systemd/system/orbien-server.service > /dev/null << 'EOF'
[Unit]
Description = orbien server
After = network.target syslog.target
Wants = network.target

[Service]
Type = simple
# Replace with the actual paths on your system
ExecStart = /path/to/orbien-server -c /path/to/orbien-server.toml

[Install]
WantedBy = multi-user.target
EOF
```

3. Enable the service to start on boot

```shell
sudo systemctl enable orbien-server
```

:::tip[Common commands]
1. Start orbien-server

```shell
sudo systemctl start orbien-server
```

2. Stop orbien-server
```shell
sudo systemctl stop orbien-server
```
3. Restart orbien-server
```shell
sudo systemctl restart orbien-server
```
4. Check orbien-server status
```shell
sudo systemctl status orbien-server
```
:::
