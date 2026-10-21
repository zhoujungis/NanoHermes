/**
 * 结果区文件预览的代码着色。
 *
 * 与消息里的 Markdown 代码块共用 lib/hl 里的同一套按需 highlight.js 实例；
 * 这里额外做三件事：从文件名推断语言、对超大文件放弃着色、以及给出行号数量。
 */
import { hljs } from "./hl";
import { escapeHtml } from "./markdown";

/** 扩展名到 highlight.js 语言标识的对照；拿不准的就不着色，保持纯文本。 */
const LANGS: Record<string, string> = {
  rs: "rust",
  ts: "typescript",
  tsx: "typescript",
  mts: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  md: "markdown",
  markdown: "markdown",
  css: "css",
  scss: "scss",
  less: "less",
  html: "xml",
  htm: "xml",
  xml: "xml",
  svg: "xml",
  py: "python",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  ps1: "powershell",
  toml: "ini",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  yml: "yaml",
  yaml: "yaml",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  hpp: "cpp",
  go: "go",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  swift: "swift",
  rb: "ruby",
  php: "php",
  cs: "csharp",
  sql: "sql",
};

/** 按文件名推断语言；没有对应关系或 common 版里没有这个语言时返回空串。 */
export function langOf(path: string): string {
  const name = path.split(/[\\/]/).pop() || "";
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  const want = LANGS[ext] || "";
  return want && hljs.getLanguage(want) ? want : "";
}

/** 超大文件不做分词：着色要遍历整段文本，收益远小于代价。 */
const MAX_HL = 200_000;

/** 着色后的 HTML（已转义，可直接写入 innerHTML）；不着色时退回纯转义文本。 */
export function highlightCode(text: string, lang: string): string {
  if (!lang || text.length > MAX_HL) return escapeHtml(text);
  try {
    return hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
  } catch {
    return escapeHtml(text);
  }
}

/** 行号数量：末尾换行会多出一个空行，不占编号；空文件也算一行。 */
export function countLines(text: string): number {
  if (!text) return 1;
  const n = text.split("\n").length;
  return text.endsWith("\n") ? Math.max(1, n - 1) : n;
}
