import { memo, useEffect, useMemo, useRef, useState } from "react";
import { rowButton } from "../lib/a11y";
import { countLines, highlightCode, langOf } from "../lib/code";
import { changeBlocks, diffLineHtml, parseDiff, wordRangesFor } from "../lib/diff";
import { copyText } from "../lib/markdown";
import {
  activeTask,
  closePreview,
  openPreview,
  setFilter,
  setPanelTab,
  useAppState,
  type Filter,
  type PanelTab,
  type Touched,
} from "../lib/store";
import { errText, invoke } from "../lib/tauri";
import { FileTree } from "./FileTree";
import { Icon, type IconName } from "./Icon";

const FILTERS: Array<[Filter, string]> = [
  ["all", "全部"],
  ["w", "已修改"],
  ["r", "已读取"],
];

const TABS: Array<[PanelTab, string]> = [
  ["tree", "文件"],
  ["artifacts", "产物"],
];

function fileIcon(path: string): IconName {
  if (/\.(rs|ts|tsx|js|jsx|py|go|java|c|cpp|h|hpp|cs|rb|php|swift|kt)$/i.test(path)) return "file";
  if (/\.(md|txt|json|toml|yaml|yml|ini|cfg|env)$/i.test(path)) return "edit";
  return "file";
}

/** 文件预览的代码视图：左侧行号固定，横向滚动只作用在代码本体上。 */
function CodeView({ path, text }: { path: string; text: string }) {
  const gutter = useRef<HTMLDivElement>(null);
  const html = useMemo(() => highlightCode(text, langOf(path)), [text, path]);
  const lines = useMemo(() => countLines(text), [text]);

  return (
    <div className="pv-code">
      <div className="pv-gutter" ref={gutter} aria-hidden="true">
        {Array.from({ length: lines }, (_, i) => (
          <span key={i}>{i + 1}</span>
        ))}
      </div>
      <div
        className="pv-scroll"
        onScroll={(e) => {
          const g = gutter.current;
          if (g) g.scrollTop = e.currentTarget.scrollTop;
        }}
      >
        <pre dangerouslySetInnerHTML={{ __html: html }} />
      </div>
    </div>
  );
}

/** 右侧结果区：Agent 本轮碰过的文件，以及单个文件的预览。 */
export function ResultsPanel({
  hidden,
  onCollapse,
}: {
  hidden: boolean;
  onCollapse: () => void;
}) {
  const st = useAppState();
  const files = activeTask()?.files ?? [];
  const preview = st.preview;
  const tab = st.panelTab;
  const shown = files
    .filter((f) => st.filter === "all" || f.kind === st.filter)
    .slice()
    .sort((a, b) => (b.ts || 0) - (a.ts || 0));

  return (
    <aside className={"aside" + (hidden ? " collapsed" : "")}>
      <div className="as-head">
        <div className="as-tabs" role="tablist" aria-label="结果区视图">
          {TABS.map(([value, label]) => (
            <button
              key={value}
              role="tab"
              aria-selected={tab === value}
              className={tab === value ? "on" : ""}
              onClick={() => setPanelTab(value)}
            >
              {label}
              {value === "artifacts" && files.length > 0 && <span className="n">{files.length}</span>}
            </button>
          ))}
        </div>
        <span className="sp" />
        <button className="tbtn" title="收起结果区" onClick={onCollapse}>
          <Icon name="x" />
        </button>
      </div>

      {tab === "artifacts" && (
        <div className="as-filters" role="group" aria-label="文件筛选">
          {FILTERS.map(([value, label]) => (
            <button
              key={value}
              aria-pressed={st.filter === value}
              className={"fchip" + (st.filter === value ? " on" : "")}
              onClick={() => setFilter(value)}
            >
              {label}
            </button>
          ))}
        </div>
      )}

      <div className={"as-list" + (preview ? " previewing" : "")}>
        {tab === "tree" ? (
          <FileTree />
        ) : shown.length === 0 ? (
          <div className="as-empty">
            <Icon name="box" />
            {files.length ? (
              "当前筛选下没有文件"
            ) : (
              <>
                本次任务还没有产生文件读取或修改。
                <br />
                Agent 操作文件时会自动汇总到这里。
              </>
            )}
          </div>
        ) : (
          shown.map((f) => (
            <FileRow key={f.path} f={f} active={preview?.path === f.path} />
          ))
        )}
      </div>

      {preview && <PreviewPane />}
    </aside>
  );
}

function FileRow({ f, active }: { f: Touched; active: boolean }) {
  const name = f.path.split(/[\\/]/).pop() || f.path;
  const dir = f.path.slice(0, f.path.length - name.length).replace(/[\\/]+$/, "");
  return (
    <div
      className={"file" + (active ? " on" : "")}
      title={f.path}
      {...rowButton(() => void openPreview(f.path))}
      onClick={() => void openPreview(f.path)}
    >
      <Icon name={fileIcon(f.path)} />
      <div className="meta">
        <div className="fn">{name}</div>
        <div className="fp">{dir || "工作区根目录"}</div>
      </div>
      <span className={"tag " + f.kind}>{f.kind === "w" ? "修改" : "读取"}</span>
    </div>
  );
}

/** 单个文件的两种视图：原始内容 / 相对 Git HEAD 的变更。 */
type DiffState = {
  loading: boolean;
  /** 工作区是否为 Git 仓库；false 时前端如实说明，不伪造差异。 */
  repo: boolean;
  text: string;
  error: string | null;
};

function PreviewPane() {
  const st = useAppState();
  const p = st.preview;
  const [copied, setCopied] = useState(false);
  const [mode, setMode] = useState<"content" | "diff">("content");
  const [diff, setDiff] = useState<DiffState | null>(null);
  const path = p?.path ?? "";

  useEffect(() => {
    if (mode !== "diff" || !path) return;
    let alive = true;
    setDiff({ loading: true, repo: true, text: "", error: null });
    invoke<{ repo: boolean; text: string }>("file_diff", { path })
      .then((r) => {
        if (alive) setDiff({ loading: false, repo: r.repo, text: r.text, error: null });
      })
      .catch((e) => {
        if (alive) setDiff({ loading: false, repo: true, text: "", error: errText(e) });
      });
    return () => {
      alive = false;
    };
  }, [mode, path]);

  if (!p) return null;

  const shown = mode === "diff" ? diff?.text ?? "" : p.text || "";

  return (
    <div className="preview">
      <div className="pv-head">
        <span className="nm" title={p.path}>
          {p.path}
        </span>
        <span className="sp" />
        <div className="seg pv-seg">
          <button className={mode === "content" ? "on" : ""} onClick={() => setMode("content")}>
            内容
          </button>
          <button className={mode === "diff" ? "on" : ""} onClick={() => setMode("diff")}>
            变更
          </button>
        </div>
        <button
          className="tbtn"
          title="复制内容"
          onClick={() => {
            copyText(shown);
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1200);
          }}
        >
          <Icon name={copied ? "check" : "copy"} />
        </button>
        <button className="tbtn" title="关闭预览" onClick={closePreview}>
          <Icon name="x" />
        </button>
      </div>
      <div className="pv-body">
        {mode === "content" ? (
          p.loading ? (
            <div className="pv-msg">正在读取…</div>
          ) : p.error ? (
            <div className="pv-msg">{p.error}</div>
          ) : p.text ? (
            <CodeView path={p.path} text={p.text} />
          ) : (
            <div className="pv-msg">（空文件）</div>
          )
        ) : (
          <DiffBody d={diff} path={p.path} />
        )}
      </div>
    </div>
  );
}

function DiffBody({ d, path }: { d: DiffState | null; path: string }) {
  if (!d || d.loading) return <div className="pv-msg">正在对比…</div>;
  if (d.error) return <div className="pv-msg">{d.error}</div>;
  if (!d.repo) return <div className="pv-msg">当前工作区不是 Git 仓库，无法生成变更对比。</div>;
  const text = d.text.replace(/\n$/, "");
  if (!text.trim()) return <div className="pv-msg">相对最近一次提交没有差异（新文件或已提交）。</div>;
  return <DiffList text={text} lang={langOf(path)} />;
}

/**
 * diff 正文：行级增删改背景 + 行内语法着色 + 同行内词级变化高亮，顶部带
 * 「上一处 / 下一处变更」跳转。着色要逐行走 hljs，结果按行缓存起来，跳转
 * 只改高亮类名，不重复解析。
 */
const DiffList = memo(function DiffList({ text, lang }: { text: string; lang: string }) {
  const lines = useMemo(() => parseDiff(text), [text]);
  const ranges = useMemo(() => wordRangesFor(lines), [lines]);
  const blocks = useMemo(() => changeBlocks(lines), [lines]);
  // 每一行属于第几个变更块；-1 表示上下文 / 元信息行。
  const blockOf = useMemo(() => {
    const map = lines.map(() => -1);
    blocks.forEach((b, bi) => {
      for (let i = b.start; i < b.end; i++) map[i] = bi;
    });
    return map;
  }, [lines, blocks]);
  // 逐行着色结果一次性算好，跳转时不重跑 hljs。
  const htmls = useMemo(
    () => lines.map((l, i) => diffLineHtml(l, lang, ranges[i])),
    [lines, lang, ranges],
  );
  /** 当前定位到的变更块下标；-1 表示还没跳过。 */
  const [cur, setCur] = useState(-1);
  const root = useRef<HTMLDivElement>(null);

  /** 跳到第 idx 个变更块（越界回绕），并把它滚到可视区中部。 */
  const jump = (idx: number) => {
    if (!blocks.length) return;
    const i = ((idx % blocks.length) + blocks.length) % blocks.length;
    setCur(i);
    requestAnimationFrame(() => {
      const el = root.current?.querySelector<HTMLElement>(`[data-ci="${i}"]`);
      // jsdom 没有布局实现，scrollIntoView 可能不存在，只在真实 WebView 里滚。
      el?.scrollIntoView?.({ block: "center" });
    });
  };

  return (
    <div className="diff-wrap" ref={root}>
      {blocks.length > 0 && (
        <div className="diff-bar">
          <span className="dt">变更</span>
          <span className="sp" />
          <span className="dcnt" title="当前变更 / 变更总数">
            {cur >= 0 ? cur + 1 : 0} / {blocks.length}
          </span>
          <button
            className="tbtn"
            title="上一处变更"
            aria-label="上一处变更"
            disabled={blocks.length < 2}
            onClick={() => jump(cur < 0 ? blocks.length - 1 : cur - 1)}
          >
            <Icon name="arrowUp" />
          </button>
          <button
            className="tbtn"
            title="下一处变更"
            aria-label="下一处变更"
            disabled={blocks.length < 2}
            onClick={() => jump(cur < 0 ? 0 : cur + 1)}
          >
            <Icon name="arrowDown" />
          </button>
        </div>
      )}
      <div className="diff">
        {lines.map((l, i) => (
          <div
            key={i}
            className={"dl d-" + l.kind + (blockOf[i] >= 0 && blockOf[i] === cur ? " cur" : "")}
            data-ci={blockOf[i] >= 0 ? blockOf[i] : undefined}
            dangerouslySetInnerHTML={{ __html: htmls[i] }}
          />
        ))}
      </div>
    </div>
  );
});
