import { useEffect, useMemo, useRef, useState } from "react";
import { closeFolderPick, confirmFolder } from "../lib/store";
import { errText, invoke } from "../lib/tauri";
import { Icon } from "./Icon";

interface Listing {
  path: string;
  parent: string | null;
  dirs: { name: string; path: string }[];
}

/** 面包屑：把绝对路径拆成可点击的层级（Windows 盘符单独处理）。 */
function crumbs(path: string): { name: string; path: string }[] {
  if (!path) return [];
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]+/);
  const out: { name: string; path: string }[] = [];
  let acc = "";
  parts.forEach((part, i) => {
    if (!part) {
      if (i === 0) acc = sep;
      return;
    }
    if (!acc) acc = part.endsWith(":") ? part + sep : part;
    else if (acc.endsWith(sep)) acc += part;
    else acc += sep + part;
    out.push({ name: part, path: acc });
  });
  return out;
}

/**
 * 工作区文件夹选择器。
 *
 * 没有用系统对话框，而是自己列目录：这样不必新增运行时依赖与权限配置，
 * 同时仍然允许直接输入路径，行为完全真实可控。
 */
export function FolderPicker({ initial }: { initial: string }) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [input, setInput] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // 连续点目录时，只接受最后一次请求的结果，避免慢响应把路径框改回旧目录。
  const seq = useRef(0);

  const go = (path: string) => {
    const mine = ++seq.current;
    setError(null);
    setListing(null);
    invoke<Listing>("list_dirs", { path: path.trim() || null })
      .then((l) => {
        if (mine !== seq.current) return;
        setListing(l);
        setInput(l.path);
      })
      .catch((e) => {
        if (mine !== seq.current) return;
        setError(errText(e));
      });
  };

  useEffect(() => {
    go(initial);
    inputRef.current?.focus();
    return () => {
      seq.current++;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initial]);

  const trail = useMemo(() => crumbs(listing?.path || ""), [listing]);

  return (
    <div className="fback" onMouseDown={closeFolderPick}>
      <div className="fpick" onMouseDown={(e) => e.stopPropagation()}>
        <div className="fp-head">
          <Icon name="folder" />
          <span>选择工作区文件夹</span>
          <button className="x" title="取消" onClick={closeFolderPick}>
            <Icon name="x" />
          </button>
        </div>

        <div className="fp-crumbs">
          {trail.length === 0 && <span className="fp-seg">此电脑</span>}
          {trail.map((c, i) => (
            <span key={c.path}>
              {i > 0 && <span className="fp-sep">/</span>}
              <button className="fp-seg" title={c.path} onClick={() => go(c.path)}>
                {c.name}
              </button>
              {i === trail.length - 1 && <span className="fp-sep">/</span>}
            </span>
          ))}
        </div>

        <div className="fp-input">
          <input
            ref={inputRef}
            value={input}
            spellCheck={false}
            placeholder="也可以直接输入路径，回车打开"
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") go(input);
            }}
          />
          <button className="ghost" onClick={() => go(input)}>
            打开
          </button>
        </div>

        <div className="fp-list">
          {error && <div className="pv-msg">{error}</div>}
          {!error && listing === null && <div className="pv-msg">正在读取目录…</div>}
          {!error && listing && (
            <>
              {listing.parent && (
                <button className="up" onClick={() => go(listing.parent as string)}>
                  <Icon name="arrowUp" />
                  <span>上一级</span>
                </button>
              )}
              {listing.dirs.map((d) => (
                <button key={d.path} title={d.path} onClick={() => go(d.path)}>
                  <Icon name="folder" />
                  <span>{d.name}</span>
                </button>
              ))}
              {listing.dirs.length === 0 && listing.path && (
                <div className="pv-msg">这个文件夹里没有子文件夹</div>
              )}
            </>
          )}
        </div>

        <div className="fp-foot">
          <span className="fp-cur" title={listing?.path || input}>
            {listing?.path || input || "未选择"}
          </span>
          <button
            className="primary"
            disabled={!listing?.path}
            onClick={() => listing && void confirmFolder(listing.path)}
          >
            选择此文件夹
          </button>
        </div>
      </div>
    </div>
  );
}
