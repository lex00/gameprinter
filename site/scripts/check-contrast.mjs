// Runs after `astro build` (npm run build) and fails it when a colour pair of
// the site's one look drops under WCAG AA: 4.5:1 for text, 3:1 for outlines
// and other UI. It reads the tokens of the shared sheet
// (scripts/web/public/arcade.css, which src/styles/game-printer.css imports)
// and the :root block of game-printer.css that sets --sl-color-white, and
// follows a var() from one token to the next, so a token changed in either
// file is checked here. Ratios are WCAG 2 relative luminance contrast.
// Adapted from plunge's site/scripts/check-contrast.mjs.
import { readFileSync } from 'node:fs';

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const shared = read('../../scripts/web/public/arcade.css');
const site = read('../src/styles/game-printer.css');

if (!/@import\s+['"]\.\.\/\.\.\/\.\.\/scripts\/web\/public\/arcade\.css['"]/.test(site)) {
	console.error('contrast: game-printer.css no longer imports scripts/web/public/arcade.css');
	process.exit(1);
}

const block = (css, name, has) => {
	for (const m of css.matchAll(/^:root\s*\{([^}]*)\}/gm)) if (m[1].includes(has)) return m[1];
	throw new Error(`${name}: no :root block sets ${has}`);
};
const raw = Object.fromEntries(
	[block(shared, 'arcade.css', '--bg'), block(site, 'game-printer.css', '--sl-color-white')].flatMap((body) =>
		[...body.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]),
	),
);
const token = (name, seen = []) => {
	const value = raw[name];
	if (!value || seen.includes(name)) return undefined;
	if (/^#[0-9a-fA-F]{6}$/.test(value)) return value;
	const ref = value.match(/^var\((--[\w-]+)\)$/);
	return ref ? token(ref[1], [...seen, name]) : undefined;
};

const lum = (hex) => {
	const [r, g, b] = [1, 3, 5].map((i) => {
		const c = parseInt(hex.slice(i, i + 2), 16) / 255;
		return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
	});
	return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
	const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
	return (hi + 0.05) / (lo + 0.05);
};

// [foreground, background, floor, what]
const PAIRS = [
	['--sl-color-white', '--sl-color-black', 4.5, 'headings on the page'],
	['--sl-color-gray-2', '--sl-color-black', 4.5, 'body text on the page'],
	['--sl-color-gray-2', '--sl-color-gray-6', 4.5, 'body text on a card'],
	['--sl-color-gray-2', '--sl-color-bg-sidebar', 4.5, 'sidebar links'],
	['--sl-color-gray-1', '--gp-chip', 4.5, 'inline code on its chip'],
	['--sl-color-gray-1', '--sl-color-gray-6', 4.5, 'code on a code block'],
	['--sl-color-gray-3', '--sl-color-black', 4.5, 'secondary text on the page'],
	['--sl-color-gray-3', '--sl-color-gray-6', 4.5, 'secondary text on a card'],
	['--sl-color-accent-high', '--sl-color-black', 4.5, 'links on the page'],
	['--sl-color-accent-high', '--sl-color-gray-6', 4.5, 'links on a card'],
	['--cyan', '--sl-color-black', 4.5, 'cyan headings and links on the page'],
	['--cyan', '--card', 4.5, 'cyan table heads and aside titles on a card'],
	['--yellow', '--sl-color-black', 4.5, 'yellow headings on the page'],
	['--yellow', '--card', 4.5, 'caution titles on a card'],
	['--green', '--card', 4.5, 'tip titles on a card'],
	['--red', '--card', 4.5, 'danger titles on a card'],
	['--pink', '--sl-color-black', 3, 'pink outlines on the page'],
	['--violet', '--sl-color-black', 3, 'violet outlines on the page'],
	['--sl-color-gray-4', '--sl-color-black', 3, 'quiet marks on the page'],
	['--gp-body', '--sl-color-black', 3, 'the logo body on the page'],
	['--gp-ink', '--gp-body', 3, 'the logo paper slot on its body'],
	['--gp-paper', '--sl-color-black', 1.2, 'the logo paper on the page'],
];

let failed = 0;
for (const [fg, bg, floor, what] of PAIRS) {
	const [f, b] = [token(fg), token(bg)];
	if (!f || !b) {
		console.error(`contrast: no colour for ${f ? bg : fg}`);
		failed++;
		continue;
	}
	const r = ratio(f, b);
	const ok = r >= floor;
	if (!ok) failed++;
	const line = `contrast: ${ok ? 'ok  ' : 'FAIL'} ${r.toFixed(2)}:1 (floor ${floor}) ${fg} ${f} on ${bg} ${b}: ${what}`;
	(ok ? console.log : console.error)(line);
}
if (failed) process.exit(1);
