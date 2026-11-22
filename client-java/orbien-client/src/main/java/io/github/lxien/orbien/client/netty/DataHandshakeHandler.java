package io.github.lxien.orbien.client.netty;

import io.github.lxien.orbien.client.OrbienClientConfig;
import io.github.lxien.orbien.client.msg.MsgCodec;
import io.github.lxien.orbien.client.msg.MsgType;
import io.github.lxien.orbien.client.msg.StartDataConn;
import io.github.lxien.orbien.client.msg.WireMessage;
import io.netty.bootstrap.Bootstrap;
import io.netty.buffer.ByteBuf;
import io.netty.channel.Channel;
import io.netty.channel.ChannelFutureListener;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelInboundHandlerAdapter;
import io.netty.channel.ChannelInitializer;
import io.netty.channel.ChannelOption;
import io.netty.channel.EventLoopGroup;
import io.netty.channel.socket.SocketChannel;
import io.netty.util.ReferenceCountUtil;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public final class DataHandshakeHandler extends ChannelInboundHandlerAdapter {
    private static final Logger log = LoggerFactory.getLogger(DataHandshakeHandler.class);

    private final EventLoopGroup group;
    private final Map<String, OrbienClientConfig.TunnelConfig> byName;

    private ByteBuf cumulation;
    private boolean headerDone;

    public DataHandshakeHandler(OrbienClientConfig config, EventLoopGroup group) {
        this.group = group;
        this.byName = new ConcurrentHashMap<>();
        for (OrbienClientConfig.TunnelConfig p : config.getTunnels()) {
            if (p.getName() != null) {
                byName.put(p.getName(), p);
            }
        }
    }

    @Override
    public void channelRead(ChannelHandlerContext ctx, Object msg) {
        ByteBuf buf = (ByteBuf) msg;
        if (headerDone) {
            ReferenceCountUtil.release(buf);
            return;
        }
        if (cumulation == null) {
            cumulation = ctx.alloc().buffer(buf.readableBytes());
        }
        cumulation.writeBytes(buf);
        buf.release();

        if (cumulation.readableBytes() < 5) {
            return;
        }
        cumulation.markReaderIndex();
        byte type = cumulation.readByte();
        long len = cumulation.readUnsignedIntLE();
        if (len > MsgCodec.MAX_BODY) {
            log.error("StartDataConn body too large: {}", len);
            ctx.close();
            return;
        }
        if (cumulation.readableBytes() < len) {
            cumulation.resetReaderIndex();
            return;
        }
        byte[] body = new byte[(int) len];
        cumulation.readBytes(body);

        WireMessage wire;
        try {
            wire = MsgCodec.decode(type, body);
        } catch (Exception e) {
            log.error("failed to decode StartDataConn", e);
            ctx.close();
            return;
        }
        if (wire.type() != MsgType.START_DATA_CONN) {
            log.warn("unexpected data message type={}", (char) wire.type());
            ctx.close();
            return;
        }

        StartDataConn start = wire.body();
        ByteBuf leftover = cumulation.isReadable() ? cumulation.readRetainedSlice(cumulation.readableBytes()) : null;
        cumulation.release();
        cumulation = null;
        headerDone = true;

        bridge(ctx, start, leftover);
    }

    private void bridge(ChannelHandlerContext ctx, StartDataConn start, ByteBuf leftover) {
        if (start.error != null && !start.error.isEmpty()) {
            log.error("StartDataConn rejected: {}", start.error);
            ReferenceCountUtil.release(leftover);
            ctx.close();
            return;
        }
        OrbienClientConfig.TunnelConfig tunnel = byName.get(start.tunnelName);
        if (tunnel == null) {
            log.error("unknown tunnel name={}", start.tunnelName);
            ReferenceCountUtil.release(leftover);
            ctx.close();
            return;
        }

        Channel data = ctx.channel();
        data.config().setAutoRead(false);

        if (ctx.pipeline().get(MsgFrameEncoder.class) != null) {
            ctx.pipeline().remove(MsgFrameEncoder.class);
        }

        Bootstrap b = new Bootstrap();
        b.group(group)
                .channel(NettyEventLoopFactory.socketChannelClass())
                .option(ChannelOption.TCP_NODELAY, true)
                .option(ChannelOption.AUTO_READ, false)
                .handler(new ChannelInitializer<SocketChannel>() {
                    @Override
                    protected void initChannel(SocketChannel ch) {
                    }
                });

        b.connect(tunnel.getLocalIp(), tunnel.getLocalPort()).addListener(
                (ChannelFutureListener) f -> {
                    if (!f.isSuccess()) {
                        log.error(
                                "failed to connect local {}:{} for tunnel={}",
                                tunnel.getLocalIp(),
                                tunnel.getLocalPort(),
                                start.tunnelName,
                                f.cause());
                        ReferenceCountUtil.release(leftover);
                        data.close();
                        return;
                    }
                    Channel local = f.channel();
                    local.pipeline().addLast(new ByteRelayHandler(data));
                    ctx.pipeline().replace(
                            DataHandshakeHandler.this,
                            "data-relay",
                            new ByteRelayHandler(local));

                    int leftoverBytes = leftover == null ? 0 : leftover.readableBytes();
                    if (leftover != null) {
                        local.writeAndFlush(leftover).addListener(
                                wf -> {
                                    if (!wf.isSuccess()) {
                                        data.close();
                                        local.close();
                                    }
                                });
                    }
                    data.config().setAutoRead(true);
                    local.config().setAutoRead(true);

                    log.debug(
                            "data connection bridged tunnel={} local={}:{} leftoverBytes={}",
                            start.tunnelName,
                            tunnel.getLocalIp(),
                            tunnel.getLocalPort(),
                            leftoverBytes);
                });
    }

    @Override
    public void channelInactive(ChannelHandlerContext ctx) {
        if (cumulation != null) {
            cumulation.release();
            cumulation = null;
        }
    }

    @Override
    public void exceptionCaught(ChannelHandlerContext ctx, Throwable cause) {
        log.error("data connection error", cause);
        ctx.close();
    }
}
