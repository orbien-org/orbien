---
sidebar_position: 2
sidebar_label: Docker
title: Docker
---

# Docker 安装

## 服务端

准备 `orbien-server.toml`：

```toml
listen = "0.0.0.0:9527"

# 可选 通过域名路由
# httpGwPort = 80
# httpsGwPort = 443

# 可选：客户端鉴权
# [auth]
# token = "YOUR_TOKEN"

# 可选：Web 管理面板
[dashboard]
addr = "0.0.0.0"
port = 8020
user = "admin"
password = "123456"
```

:::warning
`dashboard.addr` 需为 `0.0.0.0`，否则宿主机无法通过端口映射访问管理面板
:::

### 方式一：配置文件启动

```shell
docker run -d --name orbien-server --restart unless-stopped \
  -p 9527:9527 \
  -p 8020:8020 \
  -v "$PWD/orbien-server.toml:/etc/orbien/orbien-server.toml:ro" \
  ghcr.io/orbien-org/orbien-server:latest
```

### 方式二：Compose启动

```yaml
# docker-compose.yaml
services:
  orbien-server:
    image: ghcr.io/orbien-org/orbien-server:latest
    container_name: orbien-server
    restart: unless-stopped
    ports:
      - "9527:9527"
      - "8020:8020"
      # - "80:80"
      # - "443:443"
    volumes:
      - ./orbien-server.toml:/etc/orbien/orbien-server.toml:ro
```

```shell
docker compose up -d
```

<div id="env">

### 方式三：用环境变量注入配置

</div>

配置里写 <code>{'{{env.NAME}}'}</code>，用 `-e` 或 Compose `environment` 传入。语法见 [环境变量](../features/env.md)。

```toml
#orbien-server.toml
listen = "{{env.ORBIEN_LISTEN:0.0.0.0:9527}}"

[auth]
token = "{{env.ORBIEN_TOKEN}}"

[dashboard]
addr = "0.0.0.0"
port = 8020
user = "{{env.DASHBOARD_USER:admin}}"
password = "{{env.DASHBOARD_PASSWORD:123456}}"
```

```yaml
# docker-compose.yaml
services:
  orbien-server:
    image: ghcr.io/orbien-org/orbien-server:latest
    container_name: orbien-server
    restart: unless-stopped
    ports:
      - "9527:9527"
      - "8020:8020"
    environment:
      ORBIEN_TOKEN: ${ORBIEN_TOKEN}
      DASHBOARD_PASSWORD: ${DASHBOARD_PASSWORD}
    volumes:
      - ./orbien-server.toml:/etc/orbien/orbien-server.toml:ro
```

```shell
export ORBIEN_TOKEN=YOUR_TOKEN
export DASHBOARD_PASSWORD=change-me
docker compose up -d
```

:::warning
字符串字段必须给占位符加引号；变量未设置且没有默认值时进程会启动失败。桌面客户端（Orbien-Desktop）不要在配置里写 <code>{'{{env.}}'}</code>。
:::

---

## 客户端

### 挂载配置

准备 `orbien.toml`：

```toml
server = "YOUR_SERVER_IP:9527"

# 若服务端开启了 Token，需保持一致
# [auth]
# token = "YOUR_TOKEN"

[[tunnels]]
name = "mysql"
protocol = "tcp"
service = "127.0.0.1:3306"
remotePort = 9000
```

### 方式一：配置文件启动

```shell
docker run -d --name orbien --restart unless-stopped \
  -v "$PWD/orbien.toml:/etc/orbien/orbien.toml:ro" \
  ghcr.io/orbien-org/orbien:latest
```

:::tip
容器内 `127.0.0.1` 是容器自己。若要穿透**宿主机**上的服务，把 `service` 改成宿主机 IP，或使用下方 host 网络
:::

**host 网络**

与宿主机共用网络，配置里可直接写 `127.0.0.1` 访问本机服务：

```shell
docker run -d --name orbien --restart unless-stopped \
  --network host \
  -v "$PWD/orbien.toml:/etc/orbien/orbien.toml:ro" \
  ghcr.io/orbien-org/orbien:latest
```

### 方式二：Compose启动

```yaml
# docker-compose.yaml
services:
  orbien:
    image: ghcr.io/orbien-org/orbien:latest
    container_name: orbien
    restart: unless-stopped
    # network_mode: host
    volumes:
      - ./orbien.toml:/etc/orbien/orbien.toml:ro
```

```shell
docker compose up -d
```
