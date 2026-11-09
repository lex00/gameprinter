import { defineCollection } from 'astro:content';
import { z } from 'astro/zod';
import { docsLoader, i18nLoader } from '@astrojs/starlight/loaders';
import { docsSchema, i18nSchema } from '@astrojs/starlight/schema';
import { promptText } from './prompts';

export const collections = {
	// Starlight reads UI strings from this collection; en.json keeps one override.
	i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),
	docs: defineCollection({
		loader: docsLoader(),
		schema: docsSchema({
			extend: z.object({
				// "Optional: hand this page to your coding agent": shown under the
				// title and carried by llms.txt. promptText appends neverLine.
				prompt: z.string().optional().transform(promptText),
			}),
		}),
	}),
};
