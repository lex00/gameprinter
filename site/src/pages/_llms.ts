// The agent index, built from the docs collection so it never drifts from the
// site. llms.txt lists every page; llms-full.txt carries their text.
import { getCollection } from 'astro:content';
import { bodyMarkdown, pageUrl } from '../markdown';

// Sidebar order. A page not listed here comes after these, by id.
const ORDER = ['improve', 'make', 'themes', 'bring', 'install', 'rom-rules', 'registry', 'contribute', 'reference/cli'];

export async function pages() {
	const docs = await getCollection('docs');
	const rank = (id: string) => {
		const i = ORDER.indexOf(id);
		return i === -1 ? ORDER.length : i;
	};
	return docs
		.sort((a, b) => rank(a.id) - rank(b.id) || a.id.localeCompare(b.id))
		.map((d) => ({
			id: d.id,
			title: d.data.title,
			description: d.data.description ?? '',
			url: pageUrl(d.id),
			prompt: d.data.prompt,
			body: bodyMarkdown(d.body ?? ''),
		}));
}

export const PREAMBLE = `# Game Printer

> Docs for people improving, making and hosting games with Game Printer, a tool that rebuilds NES games as models you can check against the original and change.

The games themselves are played at https://lex00.github.io/games/.

A page with a prompt carries it under "Optional: hand this page to your coding agent". Every prompt forbids copying ROMs or anything derived from them into the repo, bypassing the pre-commit hook, and pushing.`;
