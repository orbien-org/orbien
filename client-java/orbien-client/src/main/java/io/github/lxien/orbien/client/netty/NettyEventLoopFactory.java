package io.github.lxien.orbien.client.netty;

import io.netty.channel.EventLoopGroup;
import io.netty.channel.epoll.Epoll;
import io.netty.channel.epoll.EpollDatagramChannel;
import io.netty.channel.epoll.EpollEventLoopGroup;
import io.netty.channel.epoll.EpollServerSocketChannel;
import io.netty.channel.epoll.EpollSocketChannel;
import io.netty.channel.kqueue.KQueue;
import io.netty.channel.kqueue.KQueueDatagramChannel;
import io.netty.channel.kqueue.KQueueEventLoopGroup;
import io.netty.channel.kqueue.KQueueServerSocketChannel;
import io.netty.channel.kqueue.KQueueSocketChannel;
import io.netty.channel.nio.NioEventLoopGroup;
import io.netty.channel.socket.DatagramChannel;
import io.netty.channel.socket.ServerSocketChannel;
import io.netty.channel.socket.SocketChannel;
import io.netty.channel.socket.nio.NioDatagramChannel;
import io.netty.channel.socket.nio.NioServerSocketChannel;
import io.netty.channel.socket.nio.NioSocketChannel;
import io.netty.util.concurrent.DefaultThreadFactory;

/**
 * Netty EventLoop factory
 * <p>
 * Prefers Epoll on Linux, KQueue on macOS/BSD, and falls back to NIO on other platforms.
 *
 * @author lxien
 * @date 2026/11/22
 */
public final class NettyEventLoopFactory {

    /**
     * EventLoop thread name prefix.
     */
    private static final String THREAD_NAME_PREFIX = "orbien-client";

    /**
     * Transport available on the current platform.
     */
    private static final Transport TRANSPORT = detectTransport();

    /**
     * Transport type.
     */
    private enum Transport {

        /**
         * Native transport for Linux.
         */
        EPOLL,

        /**
         * Native transport for macOS/BSD.
         */
        KQUEUE,

        /**
         * Generic Java NIO transport.
         */
        NIO
    }

    private NettyEventLoopFactory() {
    }

    /**
     * Detects the transport available on the current platform.
     *
     * @return transport type, Epoll first, then KQueue, finally NIO.
     */
    private static Transport detectTransport() {
        if (Epoll.isAvailable()) {
            return Transport.EPOLL;
        }
        if (KQueue.isAvailable()) {
            return Transport.KQUEUE;
        }
        return Transport.NIO;
    }

    /**
     * Creates an EventLoopGroup for the current platform.
     *
     * @param threads thread count, uses the Netty default (CPU cores * 2) when less than or equal to 0.
     * @return EventLoopGroup instance.
     */
    public static EventLoopGroup eventLoopGroup(int threads) {
        DefaultThreadFactory threadFactory = new DefaultThreadFactory(THREAD_NAME_PREFIX);
        int threadCount = Math.max(threads, 0);
        return switch (TRANSPORT) {
            case EPOLL -> new EpollEventLoopGroup(threadCount, threadFactory);
            case KQUEUE -> new KQueueEventLoopGroup(threadCount, threadFactory);
            case NIO -> new NioEventLoopGroup(threadCount, threadFactory);
            default -> throw new IllegalStateException("Unsupported transport: " + TRANSPORT);
        };
    }

    /**
     * Creates an EventLoopGroup with the Netty default thread count.
     *
     * @return EventLoopGroup instance.
     */
    public static EventLoopGroup eventLoopGroup() {
        return eventLoopGroup(0);
    }

    /**
     * Gets the TCP channel class for the current platform.
     *
     * @return SocketChannel class.
     */
    public static Class<? extends SocketChannel> socketChannelClass() {
        return switch (TRANSPORT) {
            case EPOLL -> EpollSocketChannel.class;
            case KQUEUE -> KQueueSocketChannel.class;
            case NIO -> NioSocketChannel.class;
            default -> throw new IllegalStateException("Unsupported transport: " + TRANSPORT);
        };
    }

    /**
     * Gets the server channel class for the current platform.
     *
     * @return ServerSocketChannel class.
     */
    public static Class<? extends ServerSocketChannel> serverSocketChannelClass() {
        return switch (TRANSPORT) {
            case EPOLL -> EpollServerSocketChannel.class;
            case KQUEUE -> KQueueServerSocketChannel.class;
            case NIO -> NioServerSocketChannel.class;
            default -> throw new IllegalStateException("Unsupported transport: " + TRANSPORT);
        };
    }

    /**
     * Gets the UDP channel class for the current platform.
     *
     * @return DatagramChannel class.
     */
    public static Class<? extends DatagramChannel> datagramChannelClass() {
        return switch (TRANSPORT) {
            case EPOLL -> EpollDatagramChannel.class;
            case KQUEUE -> KQueueDatagramChannel.class;
            case NIO -> NioDatagramChannel.class;
            default -> throw new IllegalStateException("Unsupported transport: " + TRANSPORT);
        };
    }

}
