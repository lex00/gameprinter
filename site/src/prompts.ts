// The prompts handed to a coding agent. Every prompt ends with neverLine, so
// none can drop the rules that keep games out of the repo.
import { SITE } from '../site.config.mjs';

export { SITE };

export const neverLine =
	'Never copy a ROM, or anything read or derived from one, into a repo; never bypass the pre-commit hook with --no-verify.';

// A page's `prompt:` front matter, ending with neverLine.
export function promptText(prompt: string | undefined): string | undefined {
	const text = prompt?.trim();
	if (!text) return undefined;
	return text.includes(neverLine) ? text : `${text}\n${neverLine}`;
}
