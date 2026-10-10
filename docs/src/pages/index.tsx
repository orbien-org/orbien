import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useBaseUrl from '@docusaurus/useBaseUrl';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import Heading from '@theme/Heading';
import ThemedImage from '@theme/ThemedImage';
import Translate, {translate} from '@docusaurus/Translate';
import HomepageFeatures from '@site/src/components/HomepageFeatures';

import styles from './index.module.css';

function HeroDashboard(): ReactNode {
    const {i18n} = useDocusaurusContext();
    const isEn = i18n.currentLocale === 'en';
    const lightSrc = useBaseUrl(
        isEn ? '/img/dashboard_en.png' : '/img/dashboard.png',
    );
    const darkSrc = useBaseUrl(
        isEn ? '/img/dashboard_en_dark.png' : '/img/dashboard_dark.png',
    );

    return (
        <div className={styles.heroShot}>
            <div className={styles.heroShotGlow} aria-hidden="true" />
            <ThemedImage
                className={styles.heroShotImg}
                alt={translate({
                    id: 'homepage.hero.dashboardAlt',
                    message: 'Orbien 项目监控面板',
                })}
                width={1494}
                height={902}
                sources={{
                    light: lightSrc,
                    dark: darkSrc,
                }}
            />
        </div>
    );
}

function HomepageHeader(): ReactNode {
    return (
        <header className={styles.hero}>
            <div className={styles.heroInner}>
                <div className={styles.heroCopy}>
                    <Heading as="h1" className={styles.heroBrand}>
                        <span className={styles.brandOrb}>Orb</span>
                        <span className={styles.brandRest}>ien</span>
                    </Heading>
                    <p className={styles.heroTagline}>
                        <Translate id="homepage.hero.tagline">由 Rust 与 Tokio 驱动</Translate>
                    </p>
                    <p className={styles.heroDesc}>
                        <Translate id="homepage.hero.desc">
                            轻量、高性能、安全的内网穿透，二进制体积大约 5MB
                        </Translate>
                    </p>
                    <div className={styles.heroActions}>
                        <Link
                            className={clsx('button button--lg', styles.btnPrimary)}
                            to="/docs/quickstart">
                            <Translate id="homepage.hero.ctaStart">快速开始</Translate>
                        </Link>
                        <Link
                            className={clsx('button button--lg', styles.btnSecondary)}
                            to="/docs/download">
                            <Translate id="homepage.hero.ctaDownload">下载</Translate>
                        </Link>
                        <Link
                            className={clsx('button button--lg', styles.btnGitHub)}
                            href="https://github.com/orbien-org/orbien">
                            GitHub
                        </Link>
                    </div>
                </div>
                <div className={styles.heroVisual}>
                    <HeroDashboard />
                </div>
            </div>
        </header>
    );
}

function DesktopShowcase(): ReactNode {
    const {i18n} = useDocusaurusContext();
    const isEn = i18n.currentLocale === 'en';
    const lightSrc = useBaseUrl(
        isEn ? '/img/desktop_en.gif' : '/img/desktop_zh.gif',
    );
    const darkSrc = useBaseUrl(
        isEn ? '/img/desktop_en_dark.gif' : '/img/desktop_zh_dark.gif',
    );

    return (
        <section className={styles.desktop} aria-labelledby="desktop-showcase-title">
            <div className={styles.desktopInner}>
                <div className={styles.desktopCopy}>
                    <Heading as="h2" id="desktop-showcase-title" className={styles.desktopTitle}>
                        <Translate id="homepage.desktop.title">桌面客户端</Translate>
                    </Heading>
                    <p className={styles.desktopDesc}>
                        <Translate id="homepage.desktop.desc">
                            纯 Rust 原生桌面客户端，通过可视化界面轻松管理隧道配置
                        </Translate>
                    </p>
                    <Link
                        className={clsx('button button--lg', styles.btnSecondary)}
                        to="/docs/download">
                        <Translate id="homepage.desktop.cta">下载桌面端</Translate>
                    </Link>
                </div>
                <div className={styles.desktopVisual}>
                    <div className={styles.desktopFrame}>
                        <ThemedImage
                            className={styles.desktopGif}
                            alt={translate({
                                id: 'homepage.desktop.alt',
                                message: 'Orbien Desktop 客户端演示',
                            })}
                            width={1920}
                            height={1279}
                            sources={{
                                light: lightSrc,
                                dark: darkSrc,
                            }}
                        />
                    </div>
                </div>
            </div>
        </section>
    );
}

export default function Home(): ReactNode {
    return (
        <Layout
            title={translate({
                id: 'homepage.meta.title',
                message: 'Orbien — 内网穿透',
            })}
            description={translate({
                id: 'homepage.meta.description',
                message: 'Orbien：简单、安全的内网穿透，支持多协议隧道与多传输协议',
            })}>
            <HomepageHeader />
            <main>
                <HomepageFeatures />
                <DesktopShowcase />
            </main>
        </Layout>
    );
}
