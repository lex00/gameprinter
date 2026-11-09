// Writes the command reference from ../CLI.md, which `gp self docs` generates,
// so the site's list can't drift from the tool's.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const text = readFileSync(new URL('../../CLI.md', import.meta.url), 'utf8');
const body = text.replace(/^# .*\n/, '').trim();
const front = `---\ntitle: gp commands\ndescription: Every gp command, from the tool's own list.\n---\n\n`;
mkdirSync(new URL('../src/content/docs/reference/', import.meta.url), { recursive: true });
writeFileSync(new URL('../src/content/docs/reference/cli.md', import.meta.url), `${front}${body}\n`);
