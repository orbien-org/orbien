#!/usr/bin/env node
import {mkdir, writeFile} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const REPO = 'orbien-org/orbien';
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const OUT = path.join(ROOT, 'src/data/release-manifest.json');

const headers = {
  Accept: 'application/vnd.github+json',
  'User-Agent': 'orbien-docs-release-manifest',
  'X-GitHub-Api-Version': '2022-11-28',
};
if (process.env.GITHUB_TOKEN) {
  headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
}

const res = await fetch(
  `https://api.github.com/repos/${REPO}/releases/latest`,
  {headers},
);

if (!res.ok) {
  const body = await res.text();
  console.error(
    `Failed to fetch ${REPO} releases/latest: ${res.status} ${res.statusText}`,
  );
  console.error(body.slice(0, 500));
  process.exit(1);
}

const data = await res.json();
const version = String(data.tag_name ?? '').replace(/^v/, '');
if (!version) {
  console.error('Release response missing tag_name');
  process.exit(1);
}

const assets = {};
for (const asset of data.assets ?? []) {
  if (typeof asset?.name === 'string' && typeof asset?.size === 'number') {
    assets[asset.name] = asset.size;
  }
}

const manifest = {
  version,
  fetchedAt: new Date().toISOString(),
  assets,
};

await mkdir(path.dirname(OUT), {recursive: true});
await writeFile(OUT, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');

console.log(
  `Wrote ${OUT} (v${version}, ${Object.keys(assets).length} assets)`,
);
