//! 长期记忆：从对话里自动提炼值得长期保留的经验/教训、用户偏好与纠正，
//! 持久化到 `~/.nanohermes/memory.json`；新会话按相关度检索回来注入系统提示词。
//!
//! 检索用的是本地 BM25 + 中日文二元切分，不引入任何需要联网拉取的依赖；
//! “语义”相关度由分词粒度与 BM25 打分近似，对中英文混合的对话足够可用。
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::config::Config;

/// 记忆条目数量上限。超出时优先淘汰旧的经验/事实，偏好与纠正更耐用。
const MAX_ENTRIES: usize = 500;
/// 单条记忆的长度上限，超长的先截断（提炼提示词也要求写短句）。
const MAX_TEXT: usize = 400;
/// 注入系统提示词的记忆条数。
pub const INJECT_TOP_K: usize = 4;
/// 注入块的字符预算，防止记忆反过来挤爆上下文。
const INJECT_CHAR_BUDGET: usize = 2400;

/// 记忆的四个类别。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 经验 / 教训：踩过的坑、验证过有效的做法。
    Lesson,
    /// 用户偏好：风格、工作流、交付方式上的稳定喜好。
    Preference,
    /// 用户纠正：被否定的做法与用户给出的正确做法。
    Correction,
    /// 稳定事实：项目约定、环境约束等长期成立的信息。
    Fact,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "lesson" | "lesson-learned" | "experience" | "lesson_learned" => Some(Kind::Lesson),
            "preference" | "pref" => Some(Kind::Preference),
            "correction" | "feedback" => Some(Kind::Correction),
            "fact" | "project" => Some(Kind::Fact),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Lesson => "lesson",
            Kind::Preference => "preference",
            Kind::Correction => "correction",
            Kind::Fact => "fact",
        }
    }

    /// 淘汰时的优先级：数字越大越先被丢弃（偏好/纠正最耐留）。
    fn eviction_rank(self) -> u8 {
        match self {
            Kind::Fact => 3,
            Kind::Lesson => 2,
            Kind::Correction => 1,
            Kind::Preference => 0,
        }
    }

    /// 注入时的中文标签。
    fn label(self) -> &'static str {
        match self {
            Kind::Lesson => "经验",
            Kind::Preference => "偏好",
            Kind::Correction => "纠正",
            Kind::Fact => "事实",
        }
    }
}

/// 一条长期记忆。
#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub text: String,
    /// 来源：`auto`（自动提炼）、`tool`（模型主动记录）或任务标识。
    pub source: String,
    /// 写入时间（Unix 秒）。
    pub ts: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 默认存储目录：与 GUI 设置、会话文件同处 `~/.nanohermes/`。
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".nanohermes"))
}

pub fn store_path(dir: &Path) -> PathBuf {
    dir.join("memory.json")
}

/// 读取记忆库；文件不存在或损坏时都退回空列表，绝不因此让对话失败。
pub fn load(dir: &Path) -> Vec<Entry> {
    let Ok(data) = std::fs::read_to_string(store_path(dir)) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&data) else {
        return Vec::new();
    };
    v.get("entries")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    let kind = Kind::parse(x.get("kind")?.as_str()?)?;
                    let text = x.get("text")?.as_str()?.trim().to_string();
                    if text.is_empty() {
                        return None;
                    }
                    Some(Entry {
                        kind,
                        text,
                        source: x
                            .get("source")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                        ts: x.get("ts").and_then(|t| t.as_u64()).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn save(dir: &Path, entries: &[Entry]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let arr: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "kind": e.kind.as_str(),
                "text": e.text,
                "source": e.source,
                "ts": e.ts,
            })
        })
        .collect();
    let data =
        serde_json::to_string_pretty(&json!({ "entries": arr })).map_err(|e| e.to_string())?;
    std::fs::write(store_path(dir), data).map_err(|e| e.to_string())
}

/// 归一化文本用于去重比较：小写、去掉空白与常见标点。
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| {
            !c.is_whitespace() && !"，。！？；：、,.!?;:()（）「」\"'“”【】…—-·*#`>~".contains(*c)
        })
        .collect()
}

/// 写入一条记忆。与已有条目重复（或一方包含另一方）时不新增，只刷新时间戳。
/// 返回 Ok(true) 表示真的新增了条目。
pub fn remember(dir: &Path, kind: Kind, text: &str, source: &str) -> Result<bool, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(false);
    }
    let text: String = text.chars().take(MAX_TEXT).collect();
    let mut entries = load(dir);
    let key = normalize(&text);
    for e in entries.iter_mut() {
        let old = normalize(&e.text);
        if old == key
            || (key.len() >= 12 && old.contains(&key))
            || (old.len() >= 12 && key.contains(&old))
        {
            e.ts = now();
            e.source = source.to_string();
            save(dir, &entries)?;
            return Ok(false);
        }
    }
    entries.push(Entry {
        kind,
        text,
        source: source.to_string(),
        ts: now(),
    });
    trim(&mut entries);
    save(dir, &entries)?;
    Ok(true)
}

/// 超出上限时淘汰：先丢最旧的经验/事实，偏好与纠正最后才动。
fn trim(entries: &mut Vec<Entry>) {
    while entries.len() > MAX_ENTRIES {
        let victim = entries
            .iter()
            .enumerate()
            // 同一重要等级下先丢最旧的（Reverse 让 max_by_key 选中较小的时间戳）。
            .max_by_key(|(_, e)| (e.kind.eviction_rank(), std::cmp::Reverse(e.ts)))
            .map(|(i, _)| i)
            .unwrap_or(0);
        entries.remove(victim);
    }
}

/* ------------------------------------------------------------------ 检索 */

/// 中英混合分词：ASCII 字母数字连续段做小写单词，CJK 连续段做二元切分
/// （单字段落保留单字）。停用词级别的单字符英文直接丢弃。
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut cjk = String::new();

    let flush_word = |buf: &mut String, out: &mut Vec<String>| {
        if buf.chars().count() >= 2 {
            out.push(buf.to_lowercase());
        }
        buf.clear();
    };
    let flush_cjk = |cjk: &mut String, out: &mut Vec<String>| {
        let chars: Vec<char> = cjk.chars().collect();
        // 单字也要进索引：查询「错」「别」这类单字必须能命中多字文档。
        // 二元词保留（精度更高），单字只作为补充召回。
        for c in &chars {
            out.push(c.to_string());
        }
        for w in chars.windows(2) {
            out.push(w.iter().collect());
        }
        cjk.clear();
    };

    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, &mut out);
            buf.push(c);
        } else if is_cjk(c) {
            flush_word(&mut buf, &mut out);
            cjk.push(c);
        } else {
            flush_word(&mut buf, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_word(&mut buf, &mut out);
    flush_cjk(&mut cjk, &mut out);
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FFF   // CJK 部首
        | 0x3040..=0x31FF // 假名 + 小假名扩展
        | 0x3400..=0x9FFF // 统一表意
        | 0xF900..=0xFAFF // 兼容表意
        | 0xAC00..=0xD7AF // 谚文
        | 0xFF66..=0xFF9D // 半角片假名
        | 0x20000..=0x2FA1F // 表意扩展 B–F
    )
}

/// BM25 打分（k1=1.2, b=0.75）。返回（分数、条目下标）按分数降序，
/// 零分（没有共同词）的条目直接过滤掉。
pub fn search(entries: &[Entry], query: &str, top_k: usize) -> Vec<(f64, usize)> {
    let q_terms = tokenize(query);
    if q_terms.is_empty() || entries.is_empty() {
        return Vec::new();
    }
    let docs: Vec<Vec<String>> = entries
        .iter()
        .map(|e| {
            let mut t = tokenize(&e.text);
            // 类别词也参与匹配：「偏好」「纠正」这样的查询能直接命中对应条目。
            t.extend(tokenize(e.kind.label()));
            t.push(e.kind.as_str().to_string());
            t
        })
        .collect();

    let n = docs.len() as f64;
    let avg_len = docs.iter().map(|d| d.len() as f64).sum::<f64>() / n.max(1.0);
    let mut df: HashMap<&str, usize> = HashMap::new();
    for d in &docs {
        let mut seen: Vec<&str> = d.iter().map(|s| s.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        for t in seen {
            *df.entry(t).or_insert(0) += 1;
        }
    }

    let k1 = 1.2f64;
    let b = 0.75f64;
    let mut scores = vec![0f64; entries.len()];

    // 逐条计算（条目数有限，无需倒排索引）。
    for (i, d) in docs.iter().enumerate() {
        let len = d.len() as f64;
        let mut tf: HashMap<&str, usize> = HashMap::new();
        for t in d {
            *tf.entry(t.as_str()).or_insert(0) += 1;
        }
        let mut score = 0f64;
        for term in &q_terms {
            let Some(f) = tf.get(term.as_str()) else {
                continue;
            };
            let f = *f as f64;
            let n_q = *df.get(term.as_str()).unwrap_or(&1) as f64;
            let idf = (1.0 + (n - n_q + 0.5) / (n_q + 0.5)).ln();
            score += idf * (f * (k1 + 1.0)) / (f + k1 * (1.0 - b + b * len / avg_len.max(1.0)));
        }
        scores[i] = score;
    }

    let mut ranked: Vec<(f64, usize)> = scores
        .into_iter()
        .enumerate()
        .filter(|(_, s)| *s > 0.0)
        .map(|(i, s)| (s, i))
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_k);
    ranked
}

/* ------------------------------------------------------- 注入系统提示词 */

/// 注入块的起止标记：会话中途重新检索时按标记整体替换，不会越积越多。
pub const BLOCK_START: &str = "=== Long-term memory (retrieved from past sessions) ===";
pub const BLOCK_END: &str = "=== End long-term memory ===";

/// 把命中的记忆渲染成一段可注入的文本；空列表返回空串。
pub fn render_block(entries: &[&Entry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from(BLOCK_START);
    out.push_str(
        "\n以下是从过往会话中检索到的相关记忆（由对话自动提炼，仅供参考；用户当前指令优先）：\n",
    );
    let mut used = 0usize;
    for e in entries {
        let line = format!("\n- [{}] {}", e.kind.label(), e.text);
        if used + line.chars().count() > INJECT_CHAR_BUDGET {
            break;
        }
        used += line.chars().count();
        out.push_str(&line);
    }
    out.push('\n');
    out.push_str(BLOCK_END);
    out
}

/// 用新的注入块替换系统消息里的旧块（没有就追加到末尾）；`block` 为空则删除旧块。
pub fn set_block(system: &str, block: &str) -> String {
    let mut base = system.to_string();
    if let Some(start) = base.find(BLOCK_START) {
        let end = base[start..]
            .find(BLOCK_END)
            .map(|i| start + i + BLOCK_END.len())
            .unwrap_or(base.len());
        base.replace_range(start..end, "");
    }
    let base = base.trim_end().to_string();
    if block.is_empty() {
        base
    } else if base.is_empty() {
        block.to_string()
    } else {
        format!("{base}\n\n{block}")
    }
}

/// 按查询检索并直接生成注入块；没有命中时返回空串。
pub fn retrieve_block(dir: &Path, query: &str) -> String {
    let entries = load(dir);
    let hits = search(&entries, query, INJECT_TOP_K);
    let picked: Vec<&Entry> = hits.iter().map(|(_, i)| &entries[*i]).collect();
    render_block(&picked)
}

/* ------------------------------------------------------------- 自动提炼 */

/// 提炼一条用户消息的廉价预判：太短的寒暄（「继续」「好的」）不值得花一次模型调用。
/// 修正类关键词直接放行——纠正与偏好恰恰是最重要的沉淀对象。
pub fn worth_distilling(user_text: &str, had_tools: bool) -> bool {
    if had_tools {
        return true;
    }
    const MARKERS: [&str; 8] = ["不对", "错了", "不是", "不要", "别", "应该", "重新", "以后"];
    let text = user_text.trim();
    if MARKERS.iter().any(|m| text.contains(m)) {
        return true;
    }
    text.chars().count() >= 24
}

const DISTILL_SYSTEM: &str = "\
You extract durable long-term memories for a coding assistant from a finished conversation turn.
Return ONLY a JSON array (no prose, no markdown fences). Each item is
{\"kind\": \"lesson|preference|correction|fact\", \"text\": \"...\"} where:
- lesson:     an experience or lesson learned (a pitfall to avoid, or an approach that worked);
- preference: a stable user preference (style, workflow, deliverable format);
- correction: something the user rejected and the right way they gave instead;
- fact:       a durable project/environment convention.
Rules: each text is ONE self-contained sentence in the conversation's language, at most 200 characters;
it must hold up in future unrelated sessions (no task-local details like dates or one-off file states);
dedupe near-identical items; if nothing is durable, return [].";

/// 让模型从一段对话摘录里提炼长期记忆。失败只影响记忆，不影响对话本身。
pub fn distill(cfg: &Config, excerpt: &str) -> Result<Vec<(Kind, String)>, String> {
    let mut cfg = cfg.clone();
    cfg.stream = false;
    let messages = vec![
        json!({ "role": "system", "content": DISTILL_SYSTEM }),
        json!({ "role": "user", "content": excerpt }),
    ];
    let turn = crate::llm::chat(&cfg, &messages, &Value::Array(vec![]), |_| {})?;
    Ok(parse_distill(&turn.content))
}

/// 解析提炼结果：容忍 ``` 围栏、前后废话、以及对象形式的 {\"memories\": [...]}；
/// 不认识的 kind 一律按「事实」处理，文本为空的条目丢弃。
pub fn parse_distill(text: &str) -> Vec<(Kind, String)> {
    let trimmed = text.trim();
    // `find` may land after `rfind` (e.g. the model writes "…] … ["), and slicing
    // a reversed range panics.
    let json_part = if let (Some(a), Some(b)) = (trimmed.find('['), trimmed.rfind(']')) {
        trimmed.get(a..=b).unwrap_or(trimmed).to_string()
    } else if let (Some(a), Some(b)) = (trimmed.find('{'), trimmed.rfind('}')) {
        trimmed.get(a..=b).unwrap_or(trimmed).to_string()
    } else {
        trimmed.to_string()
    };
    let Ok(v) = serde_json::from_str::<Value>(&json_part) else {
        return Vec::new();
    };
    let arr = match v {
        Value::Array(a) => a,
        Value::Object(_) => v
            .get("memories")
            .or_else(|| v.get("entries"))
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    arr.iter()
        .filter_map(|x| {
            let text = x.get("text").and_then(|t| t.as_str())?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let kind = x
                .get("kind")
                .and_then(|k| k.as_str())
                .and_then(Kind::parse)
                .unwrap_or(Kind::Fact);
            Some((kind, text))
        })
        .collect()
}

/// 把一截对话整理成提炼用的摘录（只留文本，工具输出截短）。
pub fn excerpt_from(messages: &[Value], max_chars: usize) -> String {
    let mut out = String::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let body = match role {
            "system" => continue,
            "tool" => format!(
                "[tool result] {}",
                content_text(m.get("content").unwrap_or(&Value::Null))
            ),
            "assistant" => {
                if m.get("tool_calls").is_some()
                    && m.get("content")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                {
                    continue;
                }
                content_text(m.get("content").unwrap_or(&Value::Null))
            }
            _ => content_text(m.get("content").unwrap_or(&Value::Null)),
        };
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        let body: String = body.chars().take(1200).collect();
        out.push_str(&format!("[{role}] {body}\n"));
        if out.chars().count() >= max_chars {
            break;
        }
    }
    out
}

/// 从消息 content（字符串或多模态数组）里抽出纯文本，忽略图片。
pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    p.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/* ------------------------------------------------------------------ 工具 */

/// `memory_save` 工具：模型把值得长期保留的经验/偏好/纠正写进记忆库。
pub fn save_tool(args: &Value, dir: &Path) -> Result<String, String> {
    let kind = args
        .get("kind")
        .and_then(|k| k.as_str())
        .and_then(Kind::parse)
        .ok_or("kind 必须是 lesson / preference / correction / fact 之一")?;
    let text = args
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or("缺少 text")?;
    let added = remember(dir, kind, text, "tool")?;
    let n = load(dir).len();
    Ok(if added {
        format!("已记下（{kind:?}），当前记忆共 {n} 条")
    } else {
        format!("与已有记忆重复，未新增；当前记忆共 {n} 条")
    })
}

/// `memory_search` 工具：模型主动翻查长期记忆。
pub fn search_tool(args: &Value, dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or("缺少 query")?;
    let top_k = args
        .get("top_k")
        .and_then(|k| k.as_u64())
        .unwrap_or(5)
        .clamp(1, 20) as usize;
    let entries = load(dir);
    if entries.is_empty() {
        return Ok("长期记忆为空。".to_string());
    }
    let hits = search(&entries, query, top_k);
    if hits.is_empty() {
        return Ok("没有相关记忆。".to_string());
    }
    let mut out = String::from("相关记忆：\n");
    for (score, i) in hits {
        let e = &entries[i];
        out.push_str(&format!(
            "- [{:.2}] [{}] {}\n",
            score,
            e.kind.label(),
            e.text
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static N: AtomicU32 = AtomicU32::new(0);

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nanohermes-memory-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(kind: Kind, text: &str) -> Entry {
        Entry {
            kind,
            text: text.to_string(),
            source: "test".to_string(),
            ts: 1,
        }
    }

    #[test]
    fn tokenize_handles_cjk_bigrams_and_ascii_words() {
        let t = tokenize("NanoHermes 前端用 Vite 构建");
        assert!(t.contains(&"nanohermes".to_string()));
        assert!(t.contains(&"vite".to_string()));
        // 连续中文按二元切分
        assert!(t.contains(&"前端".to_string()));
        assert!(t.contains(&"端用".to_string()));
    }

    #[test]
    fn round_trips_through_the_store_and_dedupes() {
        let dir = scratch();
        assert!(remember(&dir, Kind::Lesson, "测试前先跑 cargo test", "auto").unwrap());
        // 重复/近似内容不新增，只刷新时间戳
        assert!(!remember(&dir, Kind::Lesson, "测试前先跑 cargo test！", "auto").unwrap());
        assert!(!remember(&dir, Kind::Fact, "测试前先跑 cargo test 避免回归", "auto").unwrap());
        let entries = load(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, Kind::Lesson);
    }

    #[test]
    fn trims_oldest_lessons_before_touching_preferences() {
        let mut entries = vec![entry(Kind::Preference, "用户偏好简洁的中文回复")];
        for i in 0..MAX_ENTRIES {
            entries.push(entry(
                Kind::Lesson,
                &format!("经验{i}：某个只对本次有意义的细节"),
            ));
        }
        trim(&mut entries);
        assert_eq!(entries.len(), MAX_ENTRIES);
        // 偏好条目应当活下来：它被淘汰的顺位最低
        assert!(entries.iter().any(|e| e.kind == Kind::Preference));
    }

    #[test]
    fn bm25_ranks_relevant_entries_first() {
        let entries = vec![
            entry(Kind::Lesson, "构建失败时先删除 node_modules 重装"),
            entry(Kind::Fact, "前端使用 Vite 与 React"),
            entry(Kind::Preference, "回复保持简洁"),
        ];
        let hits = search(&entries, "前端构建失败", 3);
        assert!(!hits.is_empty());
        // 第 0 条命中「构建/建失/失败」多个词，第 1 条只命中「前端」，第 0 条排最前
        assert_eq!(hits[0].1, 0);
        // 无关条目不应出现
        assert!(!hits.iter().any(|(_, i)| *i == 2));
    }

    #[test]
    fn block_round_trip_replaces_instead_of_accumulating() {
        let e1 = entry(Kind::Lesson, "第一条");
        let e2 = entry(Kind::Fact, "第二条");
        let b1 = render_block(&[&e1]);
        let sys = set_block("系统提示词", &b1);
        assert!(sys.contains("系统提示词") && sys.contains("第一条"));

        let b2 = render_block(&[&e2]);
        let sys2 = set_block(&sys, &b2);
        assert!(sys2.contains("第二条"));
        assert!(!sys2.contains("第一条"), "旧块必须被整体替换");
        assert_eq!(sys2.matches(BLOCK_START).count(), 1);

        // 空块会把旧块删掉
        let sys3 = set_block(&sys2, "");
        assert!(!sys3.contains(BLOCK_START));
    }

    #[test]
    fn parses_distill_output_tolerantly() {
        let fenced = "```json\n[{\"kind\": \"preference\", \"text\": \"喜欢简短回复\"}, {\"kind\": \"weird\", \"text\": \"按事实处理\"}]\n```";
        let parsed = parse_distill(fenced);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, Kind::Preference);
        assert_eq!(parsed[1].0, Kind::Fact);

        let wrapped =
            r#"前置废话 {"memories": [{"kind": "correction", "text": "不要用 emoji"}]} 后置废话"#;
        let parsed = parse_distill(wrapped);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, Kind::Correction);

        assert!(parse_distill("没有什么可记的").is_empty());
    }

    #[test]
    fn distill_gate_skips_small_talk_but_keeps_corrections() {
        assert!(!worth_distilling("继续", false));
        assert!(!worth_distilling("好的", false));
        assert!(worth_distilling(
            "不对，界面不要用模板风格，要高级简洁",
            false
        ));
        assert!(worth_distilling(
            "请把整个项目的构建脚本与发布流程梳理一遍并写成文档",
            false
        ));
        assert!(
            worth_distilling("继续", true),
            "做过工具调用的轮次总是要提炼"
        );
    }

    #[test]
    fn tools_save_and_search_work_against_a_store() {
        let dir = scratch();
        let saved = save_tool(
            &json!({ "kind": "preference", "text": "回复使用中文" }),
            &dir,
        )
        .unwrap();
        assert!(saved.contains("已记下"));
        // 重复写入走去重分支
        let dup = save_tool(
            &json!({ "kind": "preference", "text": "回复使用中文" }),
            &dir,
        )
        .unwrap();
        assert!(dup.contains("重复"));

        let found = search_tool(&json!({ "query": "中文回复" }), &dir).unwrap();
        assert!(found.contains("回复使用中文"));

        let empty = search_tool(&json!({ "query": "完全无关的词" }), &dir).unwrap();
        assert!(empty.contains("没有相关记忆"));
    }

    #[test]
    fn excerpt_skips_system_and_images() {
        let msgs = vec![
            json!({"role": "system", "content": "系统提示词"}),
            json!({"role": "user", "content": [
                {"type": "text", "text": "帮我改样式"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,xxx"}},
            ]}),
            json!({"role": "assistant", "content": "改好了"}),
        ];
        let out = excerpt_from(&msgs, 10_000);
        assert!(out.contains("帮我改样式"));
        assert!(out.contains("改好了"));
        assert!(!out.contains("系统提示词"));
        assert!(!out.contains("base64"));
    }
}
