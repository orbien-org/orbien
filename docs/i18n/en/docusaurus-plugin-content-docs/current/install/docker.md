---
sidebar_position: 4
sidebar_label: Docker
title: Docker
---

# Docker

## Server

Prepare `orbien-server.toml`:

```toml
listen = "0.0.0.0:9527"

# optional: domain routing
# httpGwPort = 80
# httpsGwPort = 443

# optional: client authentication
# [auth]
# token = "YOUR_TOKEN"

# optional: web dashboard
[dashboard]
addr = "0.0.0.0"
port = 8020
user = "admin"
password = "123456"
```

:::warning
`dashboard.addr` must be `0.0.0.0`, otherwise the host cannot reach the dashboard through port mapping
:::

### Option 1: Start with a config file

```shell
docker run -d --name orbien-server --restart unless-stopped \
  -p 9527:9527 \
  -p 8020:8020 \
  -v "$PWD/orbien-server.toml:/etc/orbien/orbien-server.toml:ro" \
  ghcr.io/orbien-org/orbien-server:latest
```

### Option 2: Start with Compose

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

### Option 3: Inject config from environment variables

</div>

Write <code>{'{{env.NAME}}'}</code> in the config, then pass values with `-e` or Compose `environment`. See [Environment Variables](../features/env.md) for the syntax.

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
String fields must quote the placeholder. If a variable is unset and has no default, the process fails to start. Do not write <code>{'{{env.}}'}</code> in the desktop client's config.
:::

---

## Client

### Mount the config

Prepare `orbien.toml`:

```toml
server = "YOUR_SERVER_IP:9527"

# must match if the server has a token
# [auth]
# token = "YOUR_TOKEN"

[[tunnels]]
name = "mysql"
protocol = "tcp"
service = "127.0.0.1:3306"
remotePort = 9000
```

### Option 1: Start with a config file

```shell
docker run -d --name orbien --restart unless-stopped \
  -v "$PWD/orbien.toml:/etc/orbien/orbien.toml:ro" \
  ghcr.io/orbien-org/orbien:latest
```

:::tip
Inside the container, `127.0.0.1` is the container itself. To expose a service on the **host**, set `service` to the host IP, or use host networking below
:::

**Host networking**

Share the host network so the config can use `127.0.0.1` to reach local services:

```shell
docker run -d --name orbien --restart unless-stopped \
  --network host \
  -v "$PWD/orbien.toml:/etc/orbien/orbien.toml:ro" \
  ghcr.io/orbien-org/orbien:latest
```

### Option 2: Start with Compose

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
