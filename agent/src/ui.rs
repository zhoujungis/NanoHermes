use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

/// A structured event emitted by the agent while it works. External front-ends
/// (such as the desktop GUI) install a [`UiSink`] to receive these instead of
/// reading the terminal; the CLI leaves the sink unset and writes to stdout.
#[derive(Clone, Debug)]
pub enum UiEvent {
    /// A streamed assistant text delta.
    Delta(String),
    /// A tool call reported to the user: a short argument summary plus the
    /// (possibly truncated) tool result, so a GUI can show the full detail.
    ToolCall {
        name: String,
        summary: String,
        output: String,
    },
    /// A non-fatal status note (interrupt, context compaction, iteration cap).
    Note(String),
    /// Cumulative token usage after a model request, so a GUI can show what the
    /// task has cost so far. `requests` counts the model calls this update added
    /// (normally 1), so a client showing a per-task total simply sums the values
    /// it receives — which also works when the numbers come from a shared daemon.
    Usage {
        requests: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
    },
}

/// Receives [`UiEvent`]s. Must be `Send + Sync` because a single turn may run
/// tool calls (and therefore emit events) from several threads at once.
pub type UiSink = Arc<dyn Fn(UiEvent) + Send + Sync>;

/// ANSI helpers. Colour is enabled only for an interactive TTY and when
/// `NO_COLOR` is unset, so piped output stays clean. When a [`UiSink`] is
/// installed every message is delivered to it and nothing is written to the
/// terminal.
#[derive(Clone)]
pub struct Style {
    pub color: bool,
    /// When set, output is delivered here instead of stdout/stderr.
    pub sink: Option<UiSink>,
}

impl Style {
    pub fn detect() -> Style {
        let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        Style { color, sink: None }
    }

    /// A style that forwards everything to `sink` (used by embedders such as
    /// the desktop GUI, which has no terminal to write to).
    pub fn to_sink(sink: UiSink) -> Style {
        Style {
            color: false,
            sink: Some(sink),
        }
    }

    /// True when output is being forwarded to a sink rather than the terminal.
    pub fn is_silent(&self) -> bool {
        self.sink.is_some()
    }

    fn emit(&self, ev: UiEvent) -> bool {
        match &self.sink {
            Some(sink) => {
                sink(ev);
                true
            }
            None => false,
        }
    }

    fn wrap(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    pub fn dim(&self, s: &str) -> String {
        self.wrap("2", s)
    }
    pub fn bold(&self, s: &str) -> String {
        self.wrap("1", s)
    }
    pub fn cyan(&self, s: &str) -> String {
        self.wrap("36", s)
    }
    pub fn yellow(&self, s: &str) -> String {
        self.wrap("33", s)
    }
    pub fn green(&self, s: &str) -> String {
        self.wrap("32", s)
    }

    /// Report a "tool call" banner line. `output` is the tool result (forwarded
    /// to a sink for detail views; the terminal banner stays one line).
    pub fn tool_call(&self, name: &str, summary: &str, output: &str) {
        if self.emit(UiEvent::ToolCall {
            name: name.to_string(),
            summary: summary.to_string(),
            output: output.to_string(),
        }) {
            return;
        }
        println!("{} {} {}", self.cyan("→"), self.bold(name), self.dim(summary));
    }

    /// Report the token usage of one finished model request. Only graphical
    /// front-ends show this; the terminal already has `/cost`, so nothing is
    /// printed there.
    pub fn usage(&self, requests: u64, prompt_tokens: u64, completion_tokens: u64) {
        self.emit(UiEvent::Usage {
            requests,
            prompt_tokens,
            completion_tokens,
        });
    }

    /// Print a token delta without adding a newline (streaming).
    pub fn stream(&self, text: &str) {
        if self.emit(UiEvent::Delta(text.to_string())) {
            return;
        }
        print!("{text}");
        let _ = io::stdout().flush();
    }

    /// A prominent status line in the terminal (yellow). Forwarded as a
    /// [`UiEvent::Note`] when a sink is installed.
    pub fn warn(&self, msg: &str) {
        if self.emit(UiEvent::Note(msg.to_string())) {
            return;
        }
        println!("{}", self.yellow(msg));
    }

    /// A low-key status note. On an interactive terminal it is written dim to
    /// stderr; when a sink is installed it is forwarded instead.
    pub fn note(&self, msg: &str) {
        if self.emit(UiEvent::Note(msg.to_string())) {
            return;
        }
        if self.color {
            eprintln!("{}", self.dim(msg));
        }
    }
}
