package io.github.lxien.orbien.client;

import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

public final class OrbienClientConfig {
    private static final String DEFAULT_SERVER = "127.0.0.1:9527";
    private static final int DEFAULT_PORT = 9527;
    private String server = DEFAULT_SERVER;
    private String token = "";
    private boolean tcpMux = false;
    private int poolCount = 1;
    private String user = "";
    private int heartbeatIntervalSecs = 30;
    private final List<TunnelConfig> tunnels = new ArrayList<>();

    public String getServer() {
        return server;
    }

    public void setServer(String server) {
        this.server = Objects.requireNonNull(server, "server").trim();
        if (this.server.isEmpty()) {
            this.server = DEFAULT_SERVER;
        }
    }

    public String getServerHost() {
        return parseHostPort(server).host;
    }

    public int getServerPort() {
        return parseHostPort(server).port;
    }

    public String getToken() {
        return token;
    }

    public void setToken(String token) {
        this.token = token == null ? "" : token;
    }

    public boolean isTcpMux() {
        return tcpMux;
    }

    public void setTcpMux(boolean tcpMux) {
        this.tcpMux = tcpMux;
    }

    public int getPoolCount() {
        return poolCount;
    }

    public void setPoolCount(int poolCount) {
        this.poolCount = poolCount;
    }

    public String getUser() {
        return user;
    }

    public void setUser(String user) {
        this.user = user == null ? "" : user;
    }

    public int getHeartbeatIntervalSecs() {
        return heartbeatIntervalSecs;
    }

    public void setHeartbeatIntervalSecs(int heartbeatIntervalSecs) {
        this.heartbeatIntervalSecs = heartbeatIntervalSecs;
    }

    public List<TunnelConfig> getTunnels() {
        return tunnels;
    }

    static HostPort parseHostPort(String raw) {
        String s = raw == null ? "" : raw.trim();
        if (s.isEmpty()) {
            return new HostPort("127.0.0.1", DEFAULT_PORT);
        }

        if (s.startsWith("[")) {
            int close = s.indexOf(']');
            if (close < 0) {
                throw new IllegalArgumentException("invalid server address '" + raw + "': missing ']'");
            }
            String hostInner = s.substring(1, close);
            if (hostInner.isEmpty()) {
                throw new IllegalArgumentException("invalid server address '" + raw + "': empty IPv6 host");
            }
            String host = "[" + hostInner + "]";
            String after = s.substring(close + 1);
            if (after.isEmpty()) {
                return new HostPort(host, DEFAULT_PORT);
            }
            if (!after.startsWith(":")) {
                throw new IllegalArgumentException(
                        "invalid server address '" + raw + "': expected ':' after ']'");
            }
            String portStr = after.substring(1);
            if (portStr.isEmpty()) {
                return new HostPort(host, DEFAULT_PORT);
            }
            int port = parsePort(portStr, raw);
            return new HostPort(host, port == 0 ? DEFAULT_PORT : port);
        }

        int colon = s.lastIndexOf(':');
        if (colon > 0 && s.indexOf(':') == colon) {
            String host = s.substring(0, colon);
            String portStr = s.substring(colon + 1);
            if (portStr.isEmpty()) {
                return new HostPort(host, DEFAULT_PORT);
            }
            int port = parsePort(portStr, raw);
            return new HostPort(host, port == 0 ? DEFAULT_PORT : port);
        }

        return new HostPort(s, DEFAULT_PORT);
    }

    private static int parsePort(String portStr, String raw) {
        try {
            return Integer.parseInt(portStr);
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("invalid server port in '" + raw + "'", e);
        }
    }

    static final class HostPort {
        final String host;
        final int port;

        HostPort(String host, int port) {
            this.host = host;
            this.port = port;
        }
    }

    public static final class TunnelConfig {
        private String protocol = "tcp";
        private String name;
        private String localIp = "127.0.0.1";
        private int localPort;
        private int remotePort;
        private List<String> domains = new ArrayList<>();

        public String getProtocol() {
            return protocol;
        }

        public void setProtocol(String protocol) {
            this.protocol = protocol;
        }

        public String getName() {
            return name;
        }

        public void setName(String name) {
            this.name = name;
        }

        public String getLocalIp() {
            return localIp;
        }

        public void setLocalIp(String localIp) {
            this.localIp = localIp;
        }

        public int getLocalPort() {
            return localPort;
        }

        public void setLocalPort(int localPort) {
            this.localPort = localPort;
        }

        public int getRemotePort() {
            return remotePort;
        }

        public void setRemotePort(int remotePort) {
            this.remotePort = remotePort;
        }

        public List<String> getDomains() {
            return domains;
        }

        public void setDomains(List<String> domains) {
            this.domains = domains == null ? new ArrayList<>() : domains;
        }
    }
}
