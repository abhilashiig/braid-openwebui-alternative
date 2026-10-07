import MarkdownIt from 'markdown-it';

// HTML in model output is escaped (html: false), so rendered markdown cannot inject markup.
const md = new MarkdownIt({ html: false, linkify: true, breaks: false });

const defaultLink = md.renderer.rules.link_open ?? ((t, i, o, _e, s) => s.renderToken(t, i, o));
md.renderer.rules.link_open = (tokens, idx, opts, env, self) => {
	tokens[idx].attrSet('target', '_blank');
	tokens[idx].attrSet('rel', 'noopener noreferrer');
	return defaultLink(tokens, idx, opts, env, self);
};

let highlighter: { codeToHtml: (code: string, o: any) => string; getLoadedLanguages: () => string[]; loadLanguage: (l: any) => Promise<void> } | null = null;
const loadingLangs = new Set<string>();

/** Bumped when lazily-loaded highlighting or math becomes available, so callers re-render. */
export const renderState = $state({ version: 0 });

md.renderer.rules.fence = (tokens, idx) => {
	const t = tokens[idx];
	const lang = t.info.trim().split(/\s+/)[0] || 'text';
	let inner = md.utils.escapeHtml(t.content);
	if (highlighter) {
		if (highlighter.getLoadedLanguages().includes(lang)) {
			const html = highlighter.codeToHtml(t.content, { lang, themes: { light: 'github-light', dark: 'github-dark' }, defaultColor: false });
			inner = html.replace(/^<pre[^>]*><code[^>]*>/, '').replace(/<\/code><\/pre>$/, '');
		} else if (!loadingLangs.has(lang)) {
			loadingLangs.add(lang);
			highlighter
				.loadLanguage(lang)
				.then(() => renderState.version++)
				.catch(() => {});
		}
	} else {
		loadHighlighter();
	}
	return `<div class="code-block"><div class="code-head"><span>${md.utils.escapeHtml(lang)}</span><button type="button" data-copy>Copy</button></div><pre class="shiki"><code>${inner}</code></pre></div>`;
};

let highlighterPromise: Promise<void> | null = null;
function loadHighlighter() {
	highlighterPromise ??= import('shiki').then(async ({ createHighlighter, createJavaScriptRegexEngine }) => {
		// JS regex engine avoids WebAssembly, which the CSP does not allow.
		highlighter = (await createHighlighter({
			themes: ['github-light', 'github-dark'],
			langs: [],
			engine: createJavaScriptRegexEngine()
		})) as any;
		renderState.version++;
	});
}

let mathPromise: Promise<void> | null = null;
function loadMath() {
	mathPromise ??= Promise.all([import('@vscode/markdown-it-katex'), import('katex/dist/katex.min.css')]).then(([m]) => {
		// CommonJS module: the plugin may sit one or two `default`s deep depending on interop.
		const mod = m as any;
		md.use(typeof mod.default === 'function' ? mod.default : mod.default.default);
		renderState.version++;
	});
}

const MATH = /\$\$[\s\S]+?\$\$|\$[^$\s][^$\n]*\$|\\\(|\\\[/;

export function renderMarkdown(src: string): string {
	if (!mathPromise && MATH.test(src)) loadMath();
	return md.render(src);
}
