package io.github.lxien.orbien.client;

import io.github.lxien.orbien.client.auth.AuthKeys;
import io.github.lxien.orbien.client.msg.Login;
import io.github.lxien.orbien.client.msg.MsgType;
import io.github.lxien.orbien.client.msg.NewDataConn;
import io.github.lxien.orbien.client.msg.WireMessage;
import io.github.lxien.orbien.client.netty.ControlHandler;
import io.github.lxien.orbien.client.netty.MsgFrameDecoder;
import io.github.lxien.orbien.client.netty.MsgFrameEncoder;
import io.github.lxien.orbien.client.netty.DataHandshakeHandler;
import io.netty.bootstrap.Bootstrap;
import io.netty.channel.Channel;
import io.netty.channel.ChannelFuture;
import io.netty.channel.ChannelInitializer;
import io.netty.channel.ChannelOption;
import io.netty.channel.EventLoopGroup;
import io.netty.channel.nio.NioEventLoopGroup;
import io.netty.channel.socket.SocketChannel;
import io.netty.channel.socket.nio.NioSocketChannel;

import java.net.InetAddress;
import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class OrbienClient implements AutoCloseable {
    private static final Logger log = LoggerFactory.getLogger(OrbienClient.class);
    private static final String VERSION = "3.5.0";

    private final OrbienClientConfig config;
    private final AtomicBoolean started = new AtomicBoolean(false);
    private EventLoopGroup group;
    private Channel controlChannel;
    private final AtomicReference<String> sessionId = new AtomicReference<>("");

    public OrbienClient(OrbienClientConfig config) {
        this.config = config;
    }

    public OrbienClientConfig config() {
        return config;
    }

    public String sessionId() {
        return sessionId.get();
    }

    public void start() {
        if (!started.compareAndSet(false, true)) {
            return;
        }
        if (config.isTcpMux()) {
            started.set(false);
            throw new IllegalStateException("tcpMux is not supported; set tcpMux=false on client and server");
        }

        group = new NioEventLoopGroup();
        CompletableFuture<String> loginFuture = new CompletableFuture<>();
        Path sessionIdPath = resolveSessionIdPath();
        String previousSessionId = resolvePreviousSessionId(sessionIdPath);

        try {
            Bootstrap b = new Bootstrap();
            b.group(group)
                    .channel(NioSocketChannel.class)
                    .option(ChannelOption.TCP_NODELAY, true)
                    .option(ChannelOption.CONNECT_TIMEOUT_MILLIS, 10_000)
                    .handler(new ChannelInitializer<SocketChannel>() {
                        @Override
                        protected void initChannel(SocketChannel ch) {
                            ch.pipeline()
                                    .addLast(new MsgFrameDecoder())
                                    .addLast(new MsgFrameEncoder())
                                    .addLast(new ControlHandler(
                                            config,
                                            OrbienClient.this::openDataConn,
                                            id -> {
                                                sessionId.set(id);
                                                loginFuture.complete(id);
                                            },
                                            loginFuture::completeExceptionally));
                        }
                    });

            ChannelFuture cf =
                    b.connect(connectHost(config.getServerHost()), config.getServerPort()).sync();
            controlChannel = cf.channel();
            sendLogin(controlChannel, previousSessionId);

            String id = loginFuture.get(30, TimeUnit.SECONDS);
            config.setSessionId(id);
            SessionIdStore.save(sessionIdPath, id);
            log.info("connected to {} sessionId={}", config.getServer(), id);
        } catch (Exception e) {
            close();
            throw new IllegalStateException("failed to start Orbien client: " + e.getMessage(), e);
        }
    }

    private Path resolveSessionIdPath() {
        String configured = config.getSessionIdFile();
        if (configured != null && !configured.isBlank()) {
            return Path.of(configured);
        }
        return SessionIdStore.defaultPath();
    }

    private String resolvePreviousSessionId(Path sessionIdPath) {
        if (config.getSessionId() != null && !config.getSessionId().isBlank()) {
            return config.getSessionId().trim();
        }
        String loaded = SessionIdStore.load(sessionIdPath);
        if (!loaded.isEmpty()) {
            log.info("restored sessionId={} from {}", loaded, sessionIdPath);
        }
        return loaded;
    }

    private void sendLogin(Channel ch, String previousSessionId) {
        long ts = System.currentTimeMillis() / 1000;
        Login login = new Login();
        login.version = VERSION;
        login.hostname = localHostname();
        login.os = System.getProperty("os.name", "");
        login.arch = System.getProperty("os.arch", "");
        login.user = config.getUser();
        login.timestamp = ts;
        login.authDigest = AuthKeys.computeAuthDigest(config.getToken(), ts);
        login.sessionId = previousSessionId == null ? "" : previousSessionId;
        login.poolCount = Math.max(config.getPoolCount(), 1);
        ch.writeAndFlush(new WireMessage(MsgType.LOGIN, login));
        log.debug(
                "login sent hostname={} user={} poolCount={} sessionId={}",
                login.hostname,
                login.user,
                login.poolCount,
                login.sessionId.isEmpty() ? "<new>" : login.sessionId);
    }

    private void openDataConn(String currentSessionId) {
        if (group == null || group.isShuttingDown()) {
            return;
        }
        String rid = currentSessionId == null || currentSessionId.isEmpty() ? sessionId.get() : currentSessionId;
        Bootstrap b = new Bootstrap();
        b.group(group)
                .channel(NioSocketChannel.class)
                .option(ChannelOption.TCP_NODELAY, true)
                .handler(new ChannelInitializer<SocketChannel>() {
                    @Override
                    protected void initChannel(SocketChannel ch) {
                        ch.pipeline()
                                .addLast(new MsgFrameEncoder())
                                .addLast(new DataHandshakeHandler(config, group));
                    }
                });

        b.connect(connectHost(config.getServerHost()), config.getServerPort())
                .addListener(f -> {
                    if (!f.isSuccess()) {
                        log.error("failed to open data connection", f.cause());
                        return;
                    }
                    Channel data = ((ChannelFuture) f).channel();
                    long ts = System.currentTimeMillis() / 1000;
                    NewDataConn nw = new NewDataConn();
                    nw.sessionId = rid;
                    nw.timestamp = ts;
                    nw.authDigest = AuthKeys.computeAuthDigest(config.getToken(), ts);
                    data.writeAndFlush(new WireMessage(MsgType.NEW_DATA_CONN, nw));
                    log.debug("NewDataConn sent, sessionId={}", rid);
                });
    }

    private static String connectHost(String host) {
        if (host != null && host.startsWith("[") && host.endsWith("]") && host.length() > 2) {
            return host.substring(1, host.length() - 1);
        }
        return host;
    }

    private static String localHostname() {
        try {
            String h = InetAddress.getLocalHost().getHostName();
            if (h != null && !h.isBlank()) {
                return h;
            }
        } catch (Exception ignored) {
        }
        return "unknown";
    }

    @Override
    public void close() {
        started.set(false);
        if (controlChannel != null) {
            controlChannel.close();
            controlChannel = null;
        }
        if (group != null) {
            group.shutdownGracefully(0, 2, TimeUnit.SECONDS);
            group = null;
        }
    }
}
