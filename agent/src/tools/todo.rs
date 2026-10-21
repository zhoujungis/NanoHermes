//! Plan/todo tool: maintain an ordered task list for the current session so the
//! model can lay out a plan and track progress across turns.

use std::sync::Mutex;

use serde_json::Value;

use super::Ctx;

/// Upper bound on the number of items kept in the list.
const MAX_ITEMS: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    InProgress,
    Completed,
}

impl Status {
    fn parse(s: &str) -> Status {
        match s.to_ascii_lowercase().replace('_', "-").as_str() {
            "in-progress" | "active" | "doing" | "started" => Status::InProgress,
            "completed" | "complete" | "done" | "finished" => Status::Completed,
            _ => Status::Pending,
        }
    }

    fn mark(self) -> char {
        match self {
            Status::Pending => ' ',
            Status::InProgress => '~',
            Status::Completed => 'x',
        }
    }
}

#[derive(Clone)]
pub struct Todo {
    pub content: String,
    pub status: Status,
}

/// Thread-safe todo list shared across all tool invocations.
pub struct TodoList {
    items: Mutex<Vec<Todo>>,
}

impl TodoList {
    pub fn new() -> Self {
        TodoList {
            items: Mutex::new(Vec::new()),
        }
    }

    /// Replace the whole list and return its rendered form.
    pub fn replace(&self, items: Vec<Todo>) -> String {
        let mut guard = self.items.lock().unwrap();
        *guard = items;
        render(&guard)
    }

    /// Return the rendered list.
    pub fn list(&self) -> String {
        render(&self.items.lock().unwrap())
    }

    /// Number of items currently tracked.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    /// True when no items are tracked.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.items.lock().unwrap().is_empty()
    }
}

impl Default for TodoList {
    fn default() -> Self {
        Self::new()
    }
}

fn render(items: &[Todo]) -> String {
    if items.is_empty() {
        return "(no todos — create a plan with todo_write)".to_string();
    }
    let mut out = String::new();
    for (i, t) in items.iter().enumerate() {
        out.push_str(&format!("{}. [{}] {}\n", i + 1, t.status.mark(), t.content));
    }
    let done = items
        .iter()
        .filter(|t| t.status == Status::Completed)
        .count();
    out.push_str(&format!("\n{done}/{} completed", items.len()));
    out
}

/// `todo_write` — replace the task list with `todos` (array of `{content, status}`).
pub fn todo_write(args: &Value, ctx: &Ctx) -> Result<String, String> {
    let arr = args
        .get("todos")
        .and_then(|v| v.as_array())
        .ok_or("missing array field `todos`")?;
    if arr.len() > MAX_ITEMS {
        return Err(format!("too many todos ({} > {MAX_ITEMS})", arr.len()));
    }

    let mut items = Vec::with_capacity(arr.len());
    for (i, it) in arr.iter().enumerate() {
        let content = it
            .get("content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| format!("todos[{i}] is missing string field `content`"))?;
        if content.trim().is_empty() {
            return Err(format!("todos[{i}] `content` must not be empty"));
        }
        let status = it
            .get("status")
            .and_then(|s| s.as_str())
            .map(Status::parse)
            .unwrap_or(Status::Pending);
        items.push(Todo {
            content: content.trim().to_string(),
            status,
        });
    }

    let rendered = ctx.todos.replace(items);
    // Surface the plan to the user as well as to the model.
    println!("{rendered}");
    Ok(rendered)
}

/// `todo_read` — return the current task list.
pub fn todo_read(_args: &Value, ctx: &Ctx) -> Result<String, String> {
    Ok(ctx.todos.list())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> Ctx {
        Ctx {
            root: std::path::PathBuf::from("."),
            auto_approve: true,
            approval: crate::config::ApprovalPolicy::Never,
            sandbox: crate::config::SandboxMode::Off,
            max_output: 20_000,
            tasks: crate::tools::tasks::TaskManager::new(),
            todos: TodoList::new(),
            skills: Vec::new(),
            board: crate::board::Board::new(),
            gate: crate::board::Gate::new(1),
            memory_dir: None,
        }
    }

    #[test]
    fn write_then_read_round_trip() {
        let ctx = ctx();
        let out = todo_write(
            &json!({ "todos": [
                { "content": "read the file", "status": "completed" },
                { "content": "edit it", "status": "in_progress" },
                { "content": "run tests" }
            ]}),
            &ctx,
        )
        .unwrap();
        assert!(out.contains("[x] read the file"), "got: {out}");
        assert!(out.contains("[~] edit it"), "got: {out}");
        assert!(out.contains("[ ] run tests"), "got: {out}");
        assert!(out.contains("1/3 completed"), "got: {out}");

        let read = todo_read(&json!({}), &ctx).unwrap();
        assert!(read.contains("edit it"));
    }

    #[test]
    fn empty_list_and_errors() {
        let ctx = ctx();
        assert!(todo_read(&json!({}), &ctx).unwrap().contains("no todos"));

        // Missing `content`.
        assert!(todo_write(&json!({ "todos": [{ "status": "done" }] }), &ctx).is_err());
        // `todos` must be an array.
        assert!(todo_write(&json!({ "todos": "nope" }), &ctx).is_err());
    }

    #[test]
    fn status_aliases() {
        assert!(matches!(Status::parse("DONE"), Status::Completed));
        assert!(matches!(Status::parse("in_progress"), Status::InProgress));
        assert!(matches!(Status::parse("whatever"), Status::Pending));
    }
}
