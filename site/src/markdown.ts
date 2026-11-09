// A docs page as plain Markdown, for llms-full.txt and "Copy page as Markdown".
// MDX import lines and component tags are code, not text. From plunge.
import { SITE } from './prompts';

export function pageUrl(id: string): string {
	return id === 'index' ? `${SITE}/` : `${SITE}/${id}/`;
}

export function bodyMarkdown(body: string): string {
	return body
		.replace(/^(import .*|[ \t]*<\/?[A-Z][^>]*\/?>[ \t]*)$\n?/gm, '')
		.replace(/\n{3,}/g, '\n\n')
		.trim();
}

export function pageMarkdown(page: { title: string; url: string; prompt?: string; body: string }): string {
	const prompt = page.prompt ? `## Optional: hand this page to your coding agent\n\n\`\`\`text\n${page.prompt}\n\`\`\`\n\n` : '';
	return `# ${page.title}\n\nSource: ${page.url}\n\n${prompt}${page.body}`;
}
