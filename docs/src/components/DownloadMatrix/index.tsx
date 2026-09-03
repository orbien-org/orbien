import {useEffect, useMemo, useState, type ReactNode} from 'react';
import clsx from 'clsx';
import Translate, {translate} from '@docusaurus/Translate';

import styles from './styles.module.css';

const REPO = 'orbien-org/orbien';

const FALLBACK_VERSION = '3.5.0';

type OsId = 'windows' | 'linux' | 'darwin' | 'freebsd';
type ArchId = 'amd64' | 'arm64';
type LibcId = 'gnu' | 'musl';
type ProductId = 'orbien-server' | 'orbien' | 'orbien-desktop';
type AssetExt = 'tar.gz' | 'deb' | 'zip' | 'dmg';
type NoteId =
    | 'glibc'
    | 'musl'
    | 'macosGatekeeper'
    | 'none';

type Row = {
    os: OsId;
    arch: ArchId;
    libc?: LibcId;
    noteId: NoteId;
};

const RELEASE_BUILDS: ReadonlySet<string> = new Set([
    'orbien-server|linux|amd64|gnu',
    'orbien-server|linux|amd64|musl',
    'orbien-server|linux|arm64|gnu',
    'orbien-server|linux|arm64|musl',
    'orbien-server|windows|amd64',
    'orbien-server|windows|arm64',
    'orbien-server|darwin|amd64',
    'orbien-server|darwin|arm64',
    'orbien-server|freebsd|amd64',

    'orbien|linux|amd64|gnu',
    'orbien|linux|amd64|musl',
    'orbien|linux|arm64|gnu',
    'orbien|linux|arm64|musl',
    'orbien|windows|amd64',
    'orbien|windows|arm64',
    'orbien|darwin|amd64',
    'orbien|darwin|arm64',
    'orbien|freebsd|amd64',

    'orbien-desktop|linux|amd64|gnu',
    'orbien-desktop|linux|arm64|gnu',
    'orbien-desktop|windows|amd64',
    'orbien-desktop|windows|arm64',
    'orbien-desktop|darwin|amd64',
    'orbien-desktop|darwin|arm64',
]);

const ROWS: Row[] = [
    {os: 'linux', arch: 'amd64', libc: 'gnu', noteId: 'glibc'},
    {os: 'linux', arch: 'amd64', libc: 'musl', noteId: 'musl'},
    {os: 'linux', arch: 'arm64', libc: 'gnu', noteId: 'glibc'},
    {os: 'linux', arch: 'arm64', libc: 'musl', noteId: 'musl'},
    {os: 'freebsd', arch: 'amd64', noteId: 'none'},
    {os: 'windows', arch: 'amd64', noteId: 'none'},
    {os: 'windows', arch: 'arm64', noteId: 'none'},
    {os: 'darwin', arch: 'amd64', noteId: 'macosGatekeeper'},
    {os: 'darwin', arch: 'arm64', noteId: 'macosGatekeeper'},
];

const OS_LABEL: Record<OsId, string> = {
    windows: 'Windows',
    darwin: 'macOS',
    linux: 'Linux',
    freebsd: 'FreeBSD',
};

const ARCH_LABEL: Record<ArchId, string> = {
    amd64: 'amd64',
    arm64: 'arm64',
};

const PRODUCTS = [
    {
        id: 'server',
        name: 'orbien-server' as ProductId,
        label: (
            <Translate id="download.product.server">服务端</Translate>
        ),
    },
    {
        id: 'client',
        name: 'orbien' as ProductId,
        label: (
            <Translate id="download.product.client">命令行客户端</Translate>
        ),
    },
    {
        id: 'desktop',
        name: 'orbien-desktop' as ProductId,
        label: (
            <Translate id="download.product.desktop">桌面客户端</Translate>
        ),
    },
] as const;

function noteText(noteId: NoteId): string {
    switch (noteId) {
        case 'glibc':
            return translate({
                id: 'download.note.glibc',
                message: '动态链接 glibc',
            });
        case 'musl':
            return translate({
                id: 'download.note.musl',
                message: '静态链接，兼容旧 glibc',
            });
        case 'macosGatekeeper':
            return translate({
                id: 'download.note.macosGatekeeper',
                message: '如遇拦截，于系统设置中允许运行',
            });
        default:
            return '';
    }
}

function buildKey(
    product: ProductId,
    os: OsId,
    arch: ArchId,
    libc?: LibcId,
): string {
    return libc ? `${product}|${os}|${arch}|${libc}` : `${product}|${os}|${arch}`;
}

function isReleaseBuild(
    product: ProductId,
    os: OsId,
    arch: ArchId,
    libc?: LibcId,
): boolean {
    return RELEASE_BUILDS.has(buildKey(product, os, arch, libc));
}

function assetExt(product: ProductId, os: OsId): AssetExt {
    if (product !== 'orbien-desktop') {
        return 'tar.gz';
    }
    switch (os) {
        case 'linux':
            return 'deb';
        case 'windows':
            return 'zip';
        case 'darwin':
            return 'dmg';
        default:
            return 'tar.gz';
    }
}

function assetName(
    product: ProductId,
    version: string,
    os: OsId,
    arch: ArchId,
    libc?: LibcId,
): string {
    const ext = assetExt(product, os);
    if (product === 'orbien-desktop') {
        return `${product}_${version}_${os}_${arch}.${ext}`;
    }
    if (libc) {
        return `${product}_${version}_${os}_${arch}_${libc}.${ext}`;
    }
    return `${product}_${version}_${os}_${arch}.${ext}`;
}

function badgeLabel(
    product: ProductId,
    os: OsId,
    libc?: LibcId,
): string {
    const ext = assetExt(product, os);
    if (product !== 'orbien-desktop' && libc) {
        return `${libc} · ${ext}`;
    }
    return ext;
}

function assetUrl(filename: string, version: string): string {
    return `https://github.com/${REPO}/releases/download/v${version}/${filename}`;
}

function detectOs(): OsId | null {
    if (typeof navigator === 'undefined') {
        return null;
    }
    const ua = navigator.userAgent.toLowerCase();
    if (ua.includes('win')) {
        return 'windows';
    }
    if (ua.includes('mac')) {
        return 'darwin';
    }
    if (ua.includes('freebsd')) {
        return 'freebsd';
    }
    if (ua.includes('linux')) {
        return 'linux';
    }
    return null;
}

export default function DownloadMatrix(): ReactNode {
    const [version, setVersion] = useState(FALLBACK_VERSION);
    const [assetSet, setAssetSet] = useState<Set<string> | null>(null);
    const [detected, setDetected] = useState<OsId | null>(null);

    useEffect(() => {
        setDetected(detectOs());
    }, []);

    useEffect(() => {
        let cancelled = false;
        (async () => {
            try {
                const res = await fetch(
                    `https://api.github.com/repos/${REPO}/releases/latest`,
                    {headers: {Accept: 'application/vnd.github+json'}},
                );
                if (!res.ok) {
                    return;
                }
                const data = (await res.json()) as {
                    tag_name?: string;
                    assets?: { name: string }[];
                };
                if (cancelled) {
                    return;
                }
                const tag = data.tag_name?.replace(/^v/, '');
                if (tag) {
                    setVersion(tag);
                }
                if (data.assets) {
                    setAssetSet(new Set(data.assets.map((a) => a.name)));
                }
            } catch {
            }
        })();
        return () => {
            cancelled = true;
        };
    }, []);

    const osRowSpans = useMemo(() => {
        const spans = new Map<number, number>();
        let i = 0;
        while (i < ROWS.length) {
            const os = ROWS[i].os;
            let count = 1;
            while (i + count < ROWS.length && ROWS[i + count].os === os) {
                count += 1;
            }
            spans.set(i, count);
            i += count;
        }
        return spans;
    }, []);

    return (
        <div className={styles.wrap}>
            <div className={styles.toolbar}>
                <span className={styles.version}>v{version}</span>
                <a
                    className={styles.releasesLink}
                    href={`https://github.com/${REPO}/releases`}
                    rel="noopener noreferrer">
                    <Translate id="download.allReleases">所有版本</Translate>
                </a>
            </div>

            <div className={styles.tableScroll}>
                <table className={styles.table}>
                    <thead>
                    <tr>
                        <th>
                            <Translate id="download.col.os">操作系统</Translate>
                        </th>
                        <th>
                            <Translate id="download.col.arch">架构</Translate>
                        </th>
                        <th>libc</th>
                        {PRODUCTS.map((p) => (
                            <th key={p.id}>{p.label}</th>
                        ))}
                        <th>
                            <Translate id="download.col.note">说明</Translate>
                        </th>
                    </tr>
                    </thead>
                    <tbody>
                    {ROWS.map((row, idx) => {
                        const span = osRowSpans.get(idx);
                        const showOs = span !== undefined;
                        const rowLibc =
                            row.os === 'linux' ? row.libc : undefined;
                        return (
                            <tr
                                key={`${row.os}-${row.arch}-${row.libc ?? 'default'}`}
                                className={clsx(
                                    detected === row.os && styles.rowHighlight,
                                )}>
                                {showOs ? (
                                    <td rowSpan={span} className={styles.osCell}>
                                        {OS_LABEL[row.os]}
                                    </td>
                                ) : null}
                                <td>
                                    <code>{ARCH_LABEL[row.arch]}</code>
                                </td>
                                <td>
                                    {rowLibc ? (
                                        <code className={styles.libcTag}>
                                            {rowLibc}
                                        </code>
                                    ) : null}
                                </td>
                                {PRODUCTS.map((p) => {
                                    const isDesktop = p.name === 'orbien-desktop';
                                    const libcKey = isDesktop
                                        ? row.os === 'linux' &&
                                          row.libc === 'gnu'
                                            ? 'gnu'
                                            : undefined
                                        : rowLibc;
                                    const built = isReleaseBuild(
                                        p.name,
                                        row.os,
                                        row.arch,
                                        libcKey,
                                    );
                                    const label = badgeLabel(
                                        p.name,
                                        row.os,
                                        isDesktop ? undefined : rowLibc,
                                    );
                                    const file = assetName(
                                        p.name,
                                        version,
                                        row.os,
                                        row.arch,
                                        isDesktop ? undefined : rowLibc,
                                    );
                                    const published = assetSet
                                        ? assetSet.has(file)
                                        : true;
                                    const showLink = built && published;
                                    return (
                                        <td key={p.id}>
                                            {showLink ? (
                                                <a
                                                    className={styles.badge}
                                                    href={assetUrl(file, version)}
                                                    title={file}
                                                    rel="noopener noreferrer">
                                                    {label}
                                                </a>
                                            ) : null}
                                        </td>
                                    );
                                })}
                                <td className={styles.note}>
                                    {noteText(row.noteId)}
                                </td>
                            </tr>
                        );
                    })}
                    </tbody>
                </table>
            </div>

            <div className={styles.docker}>
                <h3>Docker</h3>
                <ul className={styles.dockerList}>
                    <li className={styles.dockerItem}>
                        <div className={styles.dockerMeta}>
                            <strong>orbien-server</strong>
                            <span className={styles.dockerTag}>server</span>
                        </div>
                        <code className={styles.dockerCmd}>
                            docker pull ghcr.io/orbien-org/orbien-server:latest
                        </code>
                    </li>
                    <li className={styles.dockerItem}>
                        <div className={styles.dockerMeta}>
                            <strong>orbien</strong>
                            <span className={styles.dockerTag}>client</span>
                        </div>
                        <code className={styles.dockerCmd}>
                            docker pull ghcr.io/orbien-org/orbien:latest
                        </code>
                    </li>
                </ul>
            </div>
        </div>
    );
}
