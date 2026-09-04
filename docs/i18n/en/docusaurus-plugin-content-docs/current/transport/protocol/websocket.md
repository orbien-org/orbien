---
sidebar_position: 2
sidebar_label: WebSocket
title: WebSocket
---

# WebSocket

Shares the server `listen` address with TCP. Useful when you need to traverse an HTTP proxy, or when only WebSocket is
allowed. For TLS, see [TLS](../tls.md).

## Example

Server:

```toml
# orbien-server.toml
listen = "0.0.0.0:9527"

[transport]
tcpMux = true
```

Client:

```toml
# orbien.toml
server = "YOUR_SERVER_IP:9527"

[transport]
protocol = "websocket"
tcpMux = true
heartbeatInterval = -1
wsPath = "/ws"
```

## Client parameters

| Parameter                     | Required | Default     | Description                                                                                                                  |
|-------------------------------|----------|-------------|------------------------------------------------------------------------------------------------------------------------------|
| `transport.protocol`          | Yes      | `tcp`       | Always `websocket` (or `ws`)                                                                                                 |
| `transport.wsPath`            | No       | `/~!orbien` | WebSocket HTTP path; must match the server                                                                                   |
| `transport.tcpMux`            | No       | `true`      | TCP multiplexing; must match the server                                                                                      |
| `transport.heartbeatInterval` | No       | `-1`        | Application heartbeat interval (seconds); `-1` disables it. Becomes `30` when `tcpMux` is off                                |
| `transport.heartbeatTimeout`  | No       | `-1`        | Heartbeat timeout (seconds); disconnect if no Pong. Becomes `90` when `tcpMux` is off; see [TCP Multiplexing](../tcp-mux.md) |

## Server parameters

| Parameter          | Required | Default        | Description                                |
|--------------------|----------|----------------|--------------------------------------------|
| `listen`           | Yes      | `0.0.0.0:9527` | Shared listen address with TCP             |
| `transport.wsPath` | No       | `/~!orbien`    | WebSocket HTTP path; must match the client |
| `transport.tcpMux` | No       | `true`         | TCP multiplexing; must match the client    |
