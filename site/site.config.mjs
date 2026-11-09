// Where the site is served. The site has no public home yet, so it defaults to
// the local dev server; set GP_SITE_ORIGIN and GP_SITE_BASE when it gets one.
// astro.config.mjs, src/prompts.ts and scripts/check-links.mjs all read these,
// so links, llms.txt and the link check agree.
export const ORIGIN = process.env.GP_SITE_ORIGIN ?? 'http://localhost:4321';
export const BASE = process.env.GP_SITE_BASE ?? '';
export const SITE = `${ORIGIN}${BASE}`;
