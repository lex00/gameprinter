// The Game Printer logo: a pixel-art printer with a page going in and a
// printed page coming out, on a 16x16 grid. One map drives the header logo
// (Logo.astro, coloured by theme tokens) and the favicon
// (public/brand/printer.svg, written by scripts/make-favicon.mjs).
//
// p paper, b printer body, k ink, c cyan, m magenta, y yellow, . empty
export const PIXELS = [
	'................',
	'....pppppppp....',
	'....pccppppp....',
	'....pppmmppp....',
	'....pppppyyp....',
	'....pppppppp....',
	'.bbbbbbbbbbbbbb.',
	'.bbbbbbbbbbbbbb.',
	'.bbkkkkkkkkkkbb.',
	'.bbbbbbbbbbbbbb.',
	'.bbbbbbbbbbbmbb.',
	'.bbbbbbbbbbbbbb.',
	'...pppppppppp...',
	'...pccmmyykkp...',
	'...pppppppppp...',
	'................',
];

// Runs of one colour per row, merged so the SVG stays small.
export function rects() {
	const out = [];
	PIXELS.forEach((row, y) => {
		let x = 0;
		while (x < row.length) {
			const c = row[x];
			let w = 1;
			while (row[x + w] === c) w++;
			if (c !== '.') out.push({ x, y, w, c });
			x += w;
		}
	});
	return out;
}

export function svg(colours) {
	const body = rects()
		.map((r) => `<rect x="${r.x}" y="${r.y}" width="${r.w}" height="1" fill="${colours[r.c]}"/>`)
		.join('');
	return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16" shape-rendering="crispEdges">${body}</svg>`;
}
