---
sidebar_position: 2
sidebar_label: WebSocket
title: WebSocket
---

# WebSocket

与 TCP 共用服务端 `listen`。适用于需穿透 HTTP 代理、或仅开放 WebSocket 的网络环境。TLS 配置见 [TLS](../tls.md)。

## 示例

服务端：

```toml
# orbien-server.toml
listen = "0.0.0.0:9527"

[transport]
tcpMux = true
```

客户端：

```toml
# orbien.toml
server = "YOUR_SERVER_IP:9527"

[transport]
protocol = "websocket"
tcpMux = true
heartbeatInterval = -1
wsPath = "/ws"
```

## 客户端参数

| 参数                          | 必填 | 默认值      | 说明                                                                                                  |
|-------------------------------|------|-------------|-------------------------------------------------------------------------------------------------------|
| `transport.protocol`          | 是   | `tcp`       | 固定为 `websocket`（或 `ws`）                                                                         |
| `transport.wsPath`            | 否   | `/~!orbien` | WebSocket HTTP 路径；须与服务端一致                                                                   |
| `transport.tcpMux`            | 否   | `true`      | TCP 多路复用；需与服务端一致                                                                          |
| `transport.heartbeatInterval` | 否   | `-1`        | 应用心跳间隔（秒）；`-1` 关闭。关闭 `tcpMux` 时自动变为 `30`                                          |
| `transport.heartbeatTimeout`  | 否   | `-1`        | 心跳超时（秒）；未收到 Pong 则断开。关闭 `tcpMux` 时自动变为 `90`；详见 [TCP 多路复用](../tcp-mux.md) |

## 服务端参数

| 参数               | 必填 | 默认值         | 说明                                |
|--------------------|------|----------------|-------------------------------------|
| `listen`           | 是   | `0.0.0.0:9527` | 与 TCP 共用监听地址                 |
| `transport.wsPath` | 否   | `/~!orbien`    | WebSocket HTTP 路径；须与客户端一致 |
| `transport.tcpMux` | 否   | `true`         | TCP 多路复用；需与客户端一致        |
