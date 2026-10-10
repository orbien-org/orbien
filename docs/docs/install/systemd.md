---
sidebar_position: 1
sidebar_label: systemd
title: systemd
---

## Linux

下文介绍在 Linux 操作系统通过 systemd 安装 **orbien-server** 服务端。

1. 安装 systemd

```shell
# CentOS/RHEL
yum install systemd

# Debian/Ubuntu
apt install systemd
```
2. 创建 orbien-server 服务 

```toml
sudo tee /etc/systemd/system/orbien-server.service > /dev/null << 'EOF'
[Unit]
Description = orbien server
After = network.target syslog.target
Wants = network.target

[Service]
Type = simple
# 需要修改为实际路径
ExecStart = /path/to/orbien-server -c /path/to/orbien-server.toml

[Install]
WantedBy = multi-user.target
EOF
```

3. 设置开机自启动

```shell
sudo systemctl enable orbien-server
```

:::tip[常用命令]
1. 启动 orbien-server

```shell
sudo systemctl start orbien-server
```

2. 停止 orbien-server
```shell
sudo systemctl stop orbien-server
```
3. 重启 orbien-server
```shell
sudo systemctl restart orbien-server
```
4. 查看 orbien-server 状态
```shell
sudo systemctl status orbien-server
```
:::