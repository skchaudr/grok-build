/**
 * Headless context breakdown bridge for grok-pi.
 *
 * Pi RPC does not expose system prompt / tool definition text. This extension
 * runs in-process, estimates the same raw buckets as pi-context
 * (`ceil(len/4)`), and writes JSON for the headless adapter to scale into
 * Grok's native ContextInfoBlock. No terminal UI is owned here.
 */
import { writeFileSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const COMMAND = "__pi_context_breakdown";
const OUT_ENV = "PI_GROK_CONTEXT_BREAKDOWN";

function estimateTokens(text: string | undefined | null): number {
	if (!text) return 0;
	const len = String(text).length;
	return len === 0 ? 0 : Math.ceil(len / 4);
}

function skillListingText(
	skills: Array<{
		name?: string;
		description?: string;
		filePath?: string;
		disableModelInvocation?: boolean;
	}>,
): { count: number; text: string } {
	const visible = skills.filter((skill) => !skill.disableModelInvocation);
	if (visible.length === 0) {
		return { count: 0, text: "" };
	}
	// Approximate formatSkillsForPrompt payload without reimplementing XML
	// wrappers exactly — ratio scaling absorbs constant overhead.
	const lines = visible.map((skill) =>
		[skill.name ?? "", skill.description ?? "", skill.filePath ?? ""].join("\n"),
	);
	return { count: visible.length, text: lines.join("\n") };
}

export default function (pi: ExtensionAPI) {
	pi.registerCommand(COMMAND, {
		description: "Internal Pi-Grok bridge: write context token breakdown",
		handler: async (_args, ctx) => {
			const outPath = process.env[OUT_ENV];
			if (!outPath) {
				throw new Error(`${OUT_ENV} is not set`);
			}

			const systemPrompt =
				typeof ctx.getSystemPrompt === "function" ? ctx.getSystemPrompt() : "";
			const options =
				typeof ctx.getSystemPromptOptions === "function"
					? ctx.getSystemPromptOptions()
					: ({} as {
							appendSystemPrompt?: string;
							contextFiles?: Array<{ path?: string; content?: string }>;
							skills?: Array<{
								name?: string;
								description?: string;
								filePath?: string;
								disableModelInvocation?: boolean;
							}>;
						});

			const activeNames = new Set(
				typeof pi.getActiveTools === "function" ? pi.getActiveTools() : [],
			);
			const allTools =
				typeof pi.getAllTools === "function" ? pi.getAllTools() : [];
			const activeToolDefs = allTools.filter((tool) => activeNames.has(tool.name));
			const toolDefsPayload = activeToolDefs.map((tool) => ({
				name: tool.name,
				description: tool.description,
				parameters: tool.parameters,
			}));

			const contextFiles = Array.isArray(options.contextFiles)
				? options.contextFiles.map((file) => ({
						path: String(file?.path ?? ""),
						tokensRaw: estimateTokens(file?.content ?? ""),
					}))
				: [];

			const skills = Array.isArray(options.skills) ? options.skills : [];
			const skillListing = skillListingText(skills);

			const payload = {
				version: 1,
				systemPromptTokensRaw: estimateTokens(systemPrompt),
				toolDefinitionsCount: activeToolDefs.length,
				toolDefinitionsTokensRaw: estimateTokens(JSON.stringify(toolDefsPayload)),
				appendTokensRaw: estimateTokens(options.appendSystemPrompt ?? ""),
				contextFiles,
				skillsCount: skillListing.count,
				skillsTokensRaw: estimateTokens(skillListing.text),
			};

			writeFileSync(outPath, JSON.stringify(payload), "utf8");
		},
	});
}
