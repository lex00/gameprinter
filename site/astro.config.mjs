// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { BASE, ORIGIN } from './site.config.mjs';

// Structure and look follow the plunge site (INTENTIUS/plunge, site/), which
// follows choudoufu's: Starlight, Geist, dark first, an agent-ready page title,
// llms.txt, and build checks for links and contrast.

// expressive-code renders each code line as its own block with no newline
// between them, so text copied or extracted from the page runs lines together.
// A trailing newline text node in each line fixes that without changing the
// rendering. Copied from plunge's astro.config.mjs.
const newlinesBetweenLines = {
	name: 'newlines-between-lines',
	hooks: {
		postprocessRenderedBlock: ({ renderData }) => {
			const visit = (node) => {
				if (!node.children) return;
				const lines = node.children.filter((c) => c.type === 'element' && (c.properties?.className ?? []).includes('ec-line'));
				for (const line of lines.slice(0, -1)) {
					const code = line.children.find((c) => c.type === 'element');
					if (code && !code.children.some((c) => c.type === 'element')) continue;
					line.children.push({ type: 'text', value: '\n' });
				}
				node.children.forEach(visit);
			};
			visit(renderData.blockAst);
		},
	},
};

export default defineConfig({
	site: ORIGIN,
	base: BASE || undefined,
	integrations: [
		starlight({
			title: 'Game Printer',
			// src/pages/404.astro is the 404 page.
			disable404Route: true,
			favicon: '/brand/printer.svg',
			expressiveCode: { plugins: [newlinesBetweenLines] },
			customCss: ['./src/styles/game-printer.css'],
			// The title gets "Copy page as Markdown", an llms.txt pointer and the
			// page's agent prompt; the header and footer carry the printer.
			components: {
				PageTitle: './src/components/PageTitle.astro',
				Footer: './src/components/Footer.astro',
				SocialIcons: './src/components/SocialIcons.astro',
				SiteTitle: './src/components/SiteTitle.astro',
				ThemeProvider: './src/components/ThemeProvider.astro',
				ThemeSelect: './src/components/ThemeSelect.astro',
			},
			description: 'Docs for people improving, making and hosting games with Game Printer.',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/lex00/gameprinter' }],
			sidebar: [
				{
					label: 'What do you want to do?',
					items: [
						{ label: 'Play', link: 'https://lex00.github.io/games/', attrs: { rel: 'noopener' } },
						{ label: 'Improve a game', slug: 'improve' },
						{ label: 'Make your own game', slug: 'make' },
						{ label: 'Themes and levels', slug: 'themes' },
						{ label: 'Bring in a game', slug: 'bring' },
					],
				},
				{
					label: 'For all of these',
					items: [
						{ label: 'Install gp', slug: 'install' },
						{ label: 'ROM rules', slug: 'rom-rules' },
					],
				},
				{
					label: 'Running the project',
					items: [
						{ label: 'Run a registry', slug: 'registry' },
						{ label: 'Contribute to the tool', slug: 'contribute' },
						{ label: 'gp commands', slug: 'reference/cli' },
					],
				},
			],
		}),
	],
});
