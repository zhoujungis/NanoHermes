/**
 * 结果区「变更」视图的 diff 行解析与语法着色。
 *
 * unified diff 里每一行都带一个字符的行首标记（+ / - / 空格）。渲染时要把标记
 * 与正文分开：标记只做行级配色，正文按文件语言做词法高亮，这样增删行里的
 * 关键字、字符串、注释各归各色，而不是整行一个颜色。
 *
 * 解析是纯函数，不碰 DOM，方便直接跑单测。
 */
import { highlightCode } from "./code";
import { escapeHtml } from "./markdown";

export type DiffKind = "hunk" | "meta" | "add" | "del" | "ctx";

export interface DiffLine {
  kind: DiffKind;
  /** 行首标记（+ / - / 空格）；hunk 头与元信息行没有。 */
  marker: string;
  /** 去掉行首标记后的正文。 */
  code: string;
}

/**
 * 把整段 unified diff 拆成逐行结构。
 *
 * `--- ` / `+++ ` 文件头只在第一个 @@ 之前算元信息：进了 hunk 之后，
 * 删除行内容恰好以 --- 开头的情况（内容 "--"）不会被误判成文件头。
 */
export function parseDiff(text: string): DiffLine[] {
  let inHunk = false;
  return text.split("\n").map((line) => {
    if (line.startsWith("@@")) {
      inHunk = true;
      return { kind: "hunk", marker: "", code: line };
    }
    if (!inHunk && (line.startsWith("---") || line.startsWith("+++"))) {
      return { kind: "meta", marker: "", code: line };
    }
    if (line.startsWith("diff ") || line.startsWith("index ")) {
      return { kind: "meta", marker: "", code: line };
    }
    if (line.startsWith("+")) return { kind: "add", marker: "+", code: line.slice(1) };
    if (line.startsWith("-")) return { kind: "del", marker: "-", code: line.slice(1) };
    // 上下文行的行首空格算标记，正文不带它，增删改三列才对得齐。
    return {
      kind: "ctx",
      marker: " ",
      code: line.startsWith(" ") ? line.slice(1) : line,
    };
  });
}

/**
 * 一行 diff 的着色 HTML（已转义，可直接写入 innerHTML）。
 *
 * hunk 头与元信息不是代码，只转义；其余行的正文按 `lang` 做词法高亮，
 * 行首标记包在 .dm 里单独压暗。空正文补一个零宽字符，保住行高。
 * `ranges` 给出词级变化区间时，变化的词包一层 .wd 再着色，嵌套始终合法。
 */
export function diffLineHtml(line: DiffLine, lang: string, ranges?: WordRange[]): string {
  const isCode = line.kind !== "hunk" && line.kind !== "meta";
  let body = "";
  if (!isCode) {
    body = escapeHtml(line.code);
  } else if (!ranges || ranges.length === 0) {
    body = highlightCode(line.code, lang);
  } else {
    // 逐段着色：变化段与未变化段各自独立成段，.wd 只包住变化段。
    let pos = 0;
    for (const r of ranges) {
      if (r.start > pos) body += highlightCode(line.code.slice(pos, r.start), lang);
      body += `<span class="wd">${highlightCode(line.code.slice(r.start, r.end), lang)}</span>`;
      pos = r.end;
    }
    if (pos < line.code.length) body += highlightCode(line.code.slice(pos), lang);
  }
  return `<span class="dm">${escapeHtml(line.marker)}</span>${body || "&#8203;"}`;
}

/* ------------------------------------------------------------- 词级高亮 */

/** 一段字符区间（相对行正文的 [start, end)），表示词级变化。 */
export interface WordRange {
  start: number;
  end: number;
}

/** 词法切分：空白段、标识符段、其余标点逐字符。CJK 字符天然落进标点档。 */
const TOKEN_RE = /\s+|[A-Za-z0-9_]+|[^\sA-Za-z0-9_]/g;

function tokenize(s: string): Array<{ t: string; start: number; end: number }> {
  const out: Array<{ t: string; start: number; end: number }> = [];
  for (const m of s.matchAll(TOKEN_RE)) {
    out.push({ t: m[0], start: m.index!, end: m.index! + m[0].length });
  }
  return out;
}

/**
 * 相邻的词合并成一段变化区间。变化词之间的空白连成一片（「bar baz」这种
 * 短语级改动整段一起高亮），但尾部吞进来的空白要裁掉，行尾空格不带底色。
 */
function mergeRanges(tokens: Array<{ t: string; start: number; end: number }>): WordRange[] {
  const out: WordRange[] = [];
  /** 最后一个非空白词的结束位置，用来裁掉尾部吞进来的空白。 */
  let coreEnd = 0;
  for (const tk of tokens) {
    const last = out[out.length - 1];
    if (!tk.t.trim()) {
      if (last) last.end = tk.end;
      continue;
    }
    if (last && last.end === tk.start) last.end = tk.end;
    else out.push({ start: tk.start, end: tk.end });
    coreEnd = tk.end;
  }
  if (out.length) out[out.length - 1].end = coreEnd;
  return out;
}

/**
 * 词级 diff：去掉两行的公共前后缀，中间未匹配的词即变化区间。
 *
 * 对「一行里只改几个词」的典型改动给出精确区间；整行重写时退化为整行变化，
 * 效果等同于不标词级。纯函数，不碰 DOM。
 */
export function wordDiff(a: string, b: string): { a: WordRange[]; b: WordRange[] } {
  const ta = tokenize(a);
  const tb = tokenize(b);
  let p = 0;
  while (p < ta.length && p < tb.length && ta[p].t === tb[p].t) p++;
  let s = 0;
  while (s < ta.length - p && s < tb.length - p && ta[ta.length - 1 - s].t === tb[tb.length - 1 - s].t) s++;
  return {
    a: mergeRanges(ta.slice(p, ta.length - s)),
    b: mergeRanges(tb.slice(p, tb.length - s)),
  };
}

/**
 * 给整份 diff 的每一行标注词级变化区间。
 *
 * unified diff 里一个变更块通常是「若干删除行 + 若少新增行」（或反过来），
 * 按出现顺序两两配对做词级 diff；配不上的行不标词级（整行底色已足够）。
 */
export function wordRangesFor(lines: DiffLine[]): WordRange[][] {
  const out: WordRange[][] = lines.map(() => []);
  let i = 0;
  while (i < lines.length) {
    const kind = lines[i].kind;
    if (kind !== "add" && kind !== "del") {
      i++;
      continue;
    }
    const other = kind === "add" ? "del" : "add";
    let j = i;
    while (j < lines.length && lines[j].kind === kind) j++;
    let k = j;
    while (k < lines.length && lines[k].kind === other) k++;
    const n = Math.min(j - i, k - j);
    for (let x = 0; x < n; x++) {
      const first = i + x;
      const second = j + x;
      const delIdx = kind === "del" ? first : second;
      const addIdx = kind === "add" ? first : second;
      const { a, b } = wordDiff(lines[delIdx].code, lines[addIdx].code);
      out[delIdx] = a;
      out[addIdx] = b;
    }
    i = k;
  }
  return out;
}

/* ------------------------------------------------------- 变更块（跳转用） */

export interface ChangeBlock {
  /** 块首行下标（含）。 */
  start: number;
  /** 块尾行下标 + 1（不含）。 */
  end: number;
}

/** 连续的增删行归并为一个变更块，供上一处 / 下一处跳转。 */
export function changeBlocks(lines: DiffLine[]): ChangeBlock[] {
  const blocks: ChangeBlock[] = [];
  lines.forEach((l, i) => {
    if (l.kind !== "add" && l.kind !== "del") return;
    const last = blocks[blocks.length - 1];
    if (last && last.end === i) last.end = i + 1;
    else blocks.push({ start: i, end: i + 1 });
  });
  return blocks;
}
