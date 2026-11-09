// Runs after `astro build` (npm run build) and fails it when any built page
// links an internal address that leads nowhere. It reads every href, src and
// srcset in every .html file under dist/, resolves each against the page's own
// URL, and requires every address on this host to sit under the base and to be
// a built file: a page or an asset. External addresses and anchors are not
// checked here. Adapted from plunge's site/scripts/check-links.mjs.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { BASE as base, ORIGIN as origin } from '../site.config.mjs';

const dist = fileURLToPath(new URL('../dist', import.meta.url));

if (!existsSync(dist)) {
	console.error('site/dist is missing: run astro build first');
	process.exit(1);
}

const htmlFiles = [];
const walk = (dir) => {
	for (const name of readdirSync(dir)) {
		const p = join(dir, name);
		if (statSync(p).isDirectory()) walk(p);
		else if (name.endsWith('.html')) htmlFiles.push(p);
	}
};
walk(dist);
if (htmlFiles.length === 0) {
	console.error('site/dist holds no .html files; this check would pass on nothing');
	process.exit(1);
}

/** The URL a built file is served at. */
const pageUrl = (file) => {
	const rel = relative(dist, file).split(sep).join('/');
	const path = rel === 'index.html' ? '' : rel.endsWith('/index.html') ? rel.slice(0, -'index.html'.length) : rel;
	return `${origin}${base}/${path}`;
};

/** Whether a path under the base names a built file. */
const resolves = (path) => {
	const p = join(dist, decodeURIComponent(path.slice(base.length)));
	if (existsSync(p) && statSync(p).isFile()) return true;
	return existsSync(join(p, 'index.html'));
};

const unescape = (s) => s.replace(/&amp;/g, '&').replace(/&#38;/g, '&').replace(/&quot;/g, '"');

// target -> the pages that link it
const broken = new Map();
let checked = 0;
for (const file of htmlFiles) {
	const html = readFileSync(file, 'utf8');
	const here = pageUrl(file);
	const refs = [];
	for (const [tag] of html.matchAll(/<[a-zA-Z][^>]*>/g)) {
		// GitHub Pages serves 404.html for any missing address; Starlight still
		// gives it a canonical /404/, which nobody follows.
		if (file === join(dist, '404.html') && /\srel="canonical"/.test(tag)) continue;
		for (const [, attr, value] of tag.matchAll(/\s(href|src|srcset)="([^"]*)"/g)) {
			if (attr === 'srcset') for (const part of value.split(',')) refs.push(part.trim().split(/\s+/)[0]);
			else refs.push(value);
		}
	}
	for (const raw of refs) {
		const ref = unescape(raw.trim());
		if (!ref || ref.startsWith('#') || /^(mailto|tel|data|javascript):/i.test(ref)) continue;
		let url;
		try {
			url = new URL(ref, here);
		} catch {
			continue;
		}
		if (url.origin !== origin) continue;
		const path = url.pathname;
		// A full URL elsewhere on the origin is another site, not this build. A
		// relative or root-relative one outside the base is this site's mistake.
		const full = /^https?:/i.test(ref);
		if (full && path !== base && !path.startsWith(`${base}/`)) continue;
		checked++;
		if ((path === base || path.startsWith(`${base}/`)) && resolves(path)) continue;
		const key = path.startsWith(`${base}/`) || path === base ? path : `${path} (outside ${base}/)`;
		if (!broken.has(key)) broken.set(key, new Set());
		broken.get(key).add(relative(dist, file).split(sep).join('/'));
	}
}

for (const [target, from] of [...broken].sort(([a], [b]) => a.localeCompare(b))) {
	const pages = [...from].sort();
	console.error(`links: ${target} leads nowhere, linked from ${pages.length} page(s):`);
	for (const p of pages) console.error(`  ${p}`);
}
if (broken.size) {
	console.error(`links: ${broken.size} broken internal address(es) across ${htmlFiles.length} built pages`);
	process.exit(1);
}
console.log(`links: ${checked} internal links on ${htmlFiles.length} built pages, every one resolves`);
