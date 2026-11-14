//! How each agent CLI is driven: its arguments, the flags it must have, how its output stream
//! reads, and anything it needs before a run (a sign-in check). Each is an [`AgentAdapter`]; an
//! [`AgentAdapters`] registry maps agent ids to them, and an agent it doesn't know gets the plain
//! adapter (prompt in, text out). Add an agent by implementing the trait and registering it.

use super::*;

mod aider;
mod antigravity;
mod copilot;
mod devin;
mod dsh;
mod grok;
mod hermes;
mod maki;
mod opencode;
pub use aider::Aider;
pub use antigravity::Antigravity;
pub use copilot::Copilot;
pub use devin::Devin;
pub use dsh::DeepSeekHarness;
pub use grok::Grok;
pub use hermes::Hermes;
pub use maki::Maki;
pub use opencode::OpenCode;

/// What a run's arguments are built from.
pub struct ArgsCx<'a> {
    pub options: &'a BackendOptions,
    /// The message, already protected from being read as a flag.
    pub prompt: &'a str,
    pub preamble: &'a str,
    pub mcp: &'a McpCommand,
    /// A saved session to continue (only passed to adapters that [`resume`](AgentAdapter::resumes)).
    pub resume: Option<&'a str>,
    /// Build mode: the agent may edit files; approvals and questions come to us.
    pub builder: bool,
    /// Clean room: reads go through our approval tool too, so the original stays out of reach.
    pub reads_checked: bool,
    /// A question about a build: only reading (none in Clean room), no tool server, and a fresh
    /// session or a fork that leaves the builder's session unchanged.
    pub question: bool,
}

/// One way of driving an agent CLI.
pub trait AgentAdapter: Send + Sync {
    /// The agent id this adapter is registered under (`claude-code`), plus any other names.
    fn ids(&self) -> &'static [&'static str];
    /// argv after the program for one message.
    fn args(&self, cx: &ArgsCx) -> Vec<String>;
    /// Whether this agent supplies its own live model and thinking choices.
    fn has_model_catalog(&self) -> bool {
        false
    }

    fn model_catalog_depends_on_selection(&self) -> bool {
        false
    }
    /// Query models visible in this backend's execution context, respecting cancellation.
    /// Agents without catalogue support return an empty list.
    fn model_catalog(
        &self,
        _config: &BackendConfig,
        _options: &BackendOptions,
        _directory: Option<&Path>,
        _stop: &Stop,
    ) -> Result<Vec<models::AgentModel>, ToolError> {
        Ok(Vec::new())
    }
    /// Check a saved choice in the actual run context before invoking the agent.
    fn validate_model(&self, _config: &BackendConfig, _options: &BackendOptions, _directory: Option<&Path>, _stop: &Stop) -> Result<(), ToolError> {
        Ok(())
    }
    /// Refuse options that the native transport cannot support, before preflight or process I/O.
    /// Saving a model choice and discovering its catalogue do not imply native execution support.
    /// `cwd` is the folder the agent will run in on this computer; none for a run over SSH.
    fn check_native_options(&self, _options: &BackendOptions, _cwd: Option<&Path>) -> Result<(), ToolError> {
        Ok(())
    }

    /// Use ACP by default and refuse the per-message CLI runner. An explicit `acp = false` or
    /// an SSH host must fail rather than opening an interactive agent with no approval channel.
    fn requires_acp(&self) -> bool {
        false
    }
    /// Startup arguments before the ACP flags: models, effort and the tools this access mode
    /// may expose. The default keeps the backend's extra arguments unchanged.
    fn acp_options(&self, options: &BackendOptions, _access: Access) -> Vec<String> {
        options.args.clone()
    }
    /// Session-only MCP startup flags for an agent that needs more than `session/new`'s server
    /// list. Questions never receive these flags, and the person's saved config isn't changed.
    fn acp_mcp_options(&self, _mcp: &McpCommand) -> Vec<String> {
        Vec::new()
    }
    /// Include `"type": "stdio"` on the MCP server in `session/new`. Grok's server enum requires
    /// it. Other agents leave it off, so a session that has not been checked with the field
    /// stays on the payload it already accepts.
    fn acp_mcp_stdio_type(&self) -> bool {
        false
    }
    /// Arguments after the ACP mode words and the model flag, still before a trailing subcommand.
    /// The default adds none, so agents whose flags already sit in [`acp_options`](Self::acp_options)
    /// stay unchanged.
    fn acp_mode_options(&self, _options: &BackendOptions, _access: Access) -> Vec<String> {
        Vec::new()
    }
    /// Words after every other ACP argument. Grok Build needs `stdio` last.
    fn acp_trailing(&self) -> &'static [&'static str] {
        &[]
    }
    /// Session options applied with `session/set_config_option` after the session opens, in order,
    /// before the conversation is ready. Each value is sent as a JSON string. The default sets none.
    fn acp_session_config(&self, _options: &BackendOptions) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Refuse backend options that could bypass this adapter's restrictions, before its ACP
    /// process starts. The refusal is kept on the conversation and reported by `send`.
    fn validate_options(&self, _options: &BackendOptions) -> Result<(), ToolError> {
        Ok(())
    }
    /// Refuse an access mode this agent cannot hold. The default allows every mode. A refusal
    /// is kept beside [`validate_options`](Self::validate_options) and reported by `send`.
    /// Model-list discovery sends no prompt, so it does not apply this check.
    fn validate_access(&self, _access: Access) -> Result<(), ToolError> {
        Ok(())
    }
    /// The model shown in Settings: an explicit choice or the CLI's own default. `None` means
    /// the agent picks its model and doesn't offer a model selector here.
    fn model_in_use(&self, _options: Option<&BackendOptions>) -> Option<String> {
        None
    }
    /// Models the person can choose, from a catalogue or a bounded agent query. May block:
    /// call off the UI thread. Empty means this agent offers no choices.
    fn model_choices(&self, _config: &BackendConfig, _options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(Vec::new())
    }
    /// Whether this adapter offers context and reasoning controls beyond the model selector.
    fn supports_model_tuning(&self) -> bool {
        false
    }
    /// `(help invocation, substrings it must mention)`, checked once before the first run; when
    /// they're missing the run falls back to the plain adapter. Empty: nothing to check.
    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&[], &[])
    }
    /// `program <prompt>` answers and exits, so the plain adapter is a fallback. When it opens an
    /// interactive session instead, a failed flag check keeps this adapter, and the run says what
    /// went wrong (#199).
    fn plain_fallback(&self) -> bool {
        true
    }
    /// When the flag check fails and there's no plain fallback: the error and its fix, if this
    /// agent can't work without those flags (else the run tries anyway).
    fn missing_flags_error(&self) -> Option<(&'static str, &'static str)> {
        None
    }
    /// The CLI writes one JSON event per line (else its output is the answer text).
    fn streams_json(&self) -> bool {
        true
    }
    /// Events for one parsed JSON line.
    fn parse(&self, _p: &mut StreamParser, _v: &Value) -> Vec<AgentEvent> {
        Vec::new()
    }
    /// Diagnostics that an agent reports outside its JSON stream.
    fn parse_stderr(&self, _p: &mut StreamParser, _line: &str) -> Vec<AgentEvent> {
        Vec::new()
    }
    /// It can continue a saved session.
    fn resumes(&self) -> bool {
        false
    }
    /// It honours [`ArgsCx::question`]: it can be held to reading only, using a fresh session
    /// or leaving a resumed session unchanged. Questions go only to adapters that say so.
    fn answers_questions(&self) -> bool {
        false
    }
    /// The message goes in on stdin, not in argv: a long one passes Windows' command-line limit
    /// ("The filename or extension is too long", os error 206, #402).
    fn prompt_on_stdin(&self) -> bool {
        false
    }
    /// What goes in on stdin when [`prompt_on_stdin`](AgentAdapter::prompt_on_stdin) says so:
    /// the message, with the preamble in front for an agent that takes no separate one.
    fn stdin_text(&self, _preamble: &str, prompt: &str) -> String {
        prompt.to_string()
    }
    /// The process for a local run of `program` (an adapter may clean its environment).
    fn process(&self, program: &str) -> Process {
        Process::new(program)
    }
    /// Whether it still names an account it was signed in to, so a sign-in that doesn't work any
    /// more has expired (or was signed out) rather than never happened.
    fn signed_in_before(&self) -> bool {
        false
    }
    /// Whether it's signed in, checked before a local build or connection test. `None`: can't
    /// tell; leave version-only results unchanged and let the run report any sign-in problem.
    fn signed_in(&self, _program: &str) -> Option<bool> {
        None
    }
    /// The plain-words fix when sign-in preflight or ACP startup fails. Login stays in the CLI.
    fn sign_in_fix(&self) -> Option<String> {
        None
    }
    /// Whether what a build needs from this computer is there, for an agent that's handed our tool
    /// server, checked before a build starts. An agent that isn't handed it has nothing to check.
    /// `stop` ends the check when the person stops the build.
    fn check_tool_server(&self, _stop: &CancelToken) -> Result<(), ToolError> {
        Ok(())
    }
    /// An environment variable that hands a local native run our tool server, for an agent that
    /// takes it from its environment rather than its arguments.
    fn mcp_env(&self, _mcp: &McpCommand, _access: Access) -> Result<Option<(&'static str, String)>, ToolError> {
        Ok(None)
    }
    /// Whether what a build needs on a remote SSH host is there, checked before a build starts.
    /// Default: nothing to check on the host. `stop` ends the check when the person stops the build.
    fn check_remote_tool_server(&self, _host: &str, _stop: &CancelToken) -> Result<(), ToolError> {
        Ok(())
    }
    /// Prepare this run's tool server on a remote SSH host (a config file there), returning the
    /// environment variable to set on the remote command. Default: the agent can't carry one.
    fn remote_tool_server(&self, _host: &str, _mcp: &McpCommand, _access: Access) -> Result<Option<(&'static str, String)>, ToolError> {
        Ok(None)
    }
    /// The file in which this computer keeps the agent's saved session `id`, so the project can
    /// keep a copy that travels with it. `None`: it keeps none we can copy, or not here.
    fn session_file(&self, _id: &str) -> Option<std::path::PathBuf> {
        None
    }
    /// What a new session needs to know of an earlier one, read from a kept copy of its
    /// [`session_file`](AgentAdapter::session_file): what it was asked and what it said, the
    /// newest kept, within `max` characters. `None`: nothing to tell.
    fn handoff(&self, _saved: &str, _max: usize) -> Option<String> {
        None
    }
    /// Put a kept copy of session `id` back where the agent looks for it, for a run in `cwd`,
    /// when this computer doesn't have it, so the same session carries on. `Ok(true)`: put
    /// back; `Ok(false)`: nothing to do (it's here already, or this agent keeps none).
    fn restore_session(&self, _id: &str, _saved: &[u8], _cwd: &std::path::Path) -> Result<bool, ToolError> {
        Ok(false)
    }
    /// The environment variable that moves the agent's own folder (its sign-in, settings and
    /// sessions), for [`BackendOptions::config_dir`]. `None`: it has none, and the setting is
    /// ignored.
    fn folder_var(&self) -> Option<&'static str> {
        None
    }
    /// This adapter for the agent kept in `folder` on this computer: it runs there, checks the
    /// sign-in there and keeps its sessions there. `None` when it has no such folder.
    fn in_folder(&self, _folder: std::path::PathBuf) -> Option<Arc<dyn AgentAdapter>> {
        None
    }
    /// Remove session `id` that a turn run with `program` left unfinished (stopped, failed or timed
    /// out), so nothing the agent keeps running in the background picks it up again and goes on
    /// working after we stopped it (#1115). `host`: the SSH host it ran on, None for this computer.
    /// Default: this agent leaves nothing that resumes by itself.
    fn discard_unfinished(&self, _program: &str, _id: &str, _host: Option<&str>) -> Result<(), ToolError> {
        Ok(())
    }
    /// Whether [`discard_unfinished`](Self::discard_unfinished) really deletes sessions. Only then
    /// is a turn's session recorded for a later sweep, and only then does a sweep take a record's
    /// session as deleted (the default's `Ok` deletes nothing).
    fn discards_sessions(&self) -> bool {
        false
    }
}

/// The adapters a run can use.
#[derive(Clone)]
pub struct AgentAdapters {
    adapters: Vec<Arc<dyn AgentAdapter>>,
}

impl std::fmt::Debug for AgentAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.adapters.iter().map(|a| a.ids().first().copied().unwrap_or_default())).finish()
    }
}

impl Default for AgentAdapters {
    fn default() -> Self {
        AgentAdapters::built_in()
    }
}

impl AgentAdapters {
    /// The built-in coding agents, including the ACP-only Copilot and Grok Build adapters and Devin's.
    pub fn built_in() -> AgentAdapters {
        AgentAdapters {
            adapters: vec![
                Arc::new(ClaudeCode::default()),
                Arc::new(Codex),
                Arc::new(GeminiCli),
                Arc::new(OpenCode),
                Arc::new(Antigravity),
                Arc::new(Hermes),
                Arc::new(Maki),
                Arc::new(Aider),
                Arc::new(Copilot),
                Arc::new(Grok),
                Arc::new(DeepSeekHarness),
                Arc::new(Devin),
            ],
        }
    }

    /// Add an adapter; it takes over any ids it shares with one already registered.
    pub fn register(&mut self, adapter: Arc<dyn AgentAdapter>) {
        self.adapters.retain(|a| !a.ids().iter().any(|id| adapter.ids().contains(id)));
        self.adapters.push(adapter);
    }

    /// Every registered adapter, in the order they were registered.
    pub fn all(&self) -> &[Arc<dyn AgentAdapter>] {
        &self.adapters
    }

    /// The adapter for an agent id; one it doesn't know gets the plain adapter.
    pub fn for_agent(&self, agent: &str) -> Arc<dyn AgentAdapter> {
        self.adapters.iter().find(|a| a.ids().contains(&agent)).cloned().unwrap_or_else(|| Arc::new(Plain))
    }

    /// The adapter for an agent with a backend's options: in the folder they choose for it, when
    /// it keeps one ([`AgentAdapter::in_folder`]).
    pub fn for_backend(&self, agent: &str, options: &BackendOptions) -> Arc<dyn AgentAdapter> {
        let adapter = self.for_agent(agent);
        options.config_dir_here().and_then(|dir| adapter.in_folder(dir)).unwrap_or(adapter)
    }
}

/// Any CLI: the prompt as its last argument, its output streamed as text.
pub struct Plain;

impl AgentAdapter for Plain {
    fn ids(&self) -> &'static [&'static str] {
        &[]
    }
    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        a.push(format!("{}{}", cx.preamble, cx.prompt));
        a
    }
    fn streams_json(&self) -> bool {
        false
    }
}

/// Claude Code settings with one PostToolUse hook, run after every tool call: our CLI hands over
/// the notes waiting in `dir` (`craft-anything-cli builder-notes <dir>`). Paths use forward
/// slashes and quotes so the hook's shell reads them the same on every platform.
pub fn notes_hook_settings(cli: &str, dir: &std::path::Path) -> String {
    let fwd = |s: &str| s.replace('\\', "/");
    let command = format!("\"{}\" builder-notes \"{}\"", fwd(cli), fwd(&dir.display().to_string()));
    json!({"hooks": {"PostToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": command, "timeout": 10}]}]}}).to_string()
}

/// Claude Code's tools that change something or reach outside, refused for a question.
const CHANGING_TOOLS: &[&str] =
    &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash", "PowerShell", "WebFetch", "WebSearch", "Task", "TodoWrite", "AskUserQuestion"];

/// Claude Code's tools that read files, refused outright in Clean room (#564): its defaults let it
/// read in its own folder and refuse elsewhere only until a release or a setting changes that.
const READING_TOOLS: &[&str] = &["Read", "Glob", "Grep", "LS", "NotebookRead"];

/// Claude Code answering a question about a build: the builder's session copied
/// (`--fork-session`) and the copy not kept, no tool server, and only Read, Glob and Grep (none in
/// Clean room).
fn question_args(mut a: Vec<String>, cx: &ArgsCx) -> Vec<String> {
    a.extend([
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--append-system-prompt".into(),
        cx.preamble.to_string(),
        "--model".into(),
        cx.options.cli_model.clone().unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.into()),
    ]);
    if let Some(r) = cx.resume {
        a.extend(["--resume".into(), r.to_string(), "--fork-session".into()]);
    }
    a.extend(["--no-session-persistence".into(), "--strict-mcp-config".into(), "--disallowedTools".into()]);
    a.extend(CHANGING_TOOLS.iter().map(|t| t.to_string()));
    if cx.reads_checked {
        a.extend(READING_TOOLS.iter().map(|t| t.to_string()));
    } else {
        a.extend(["--allowedTools", "Read", "Glob", "Grep"].map(String::from));
    }
    a
}

/// How Claude Code's result names a `--resume` session it doesn't have (2.1: `errors`, with no
/// turn taken). Its sessions are kept per computer and per folder, so a project copied elsewhere
/// has an id it can't find.
const NO_SUCH_SESSION: &str = "No conversation found with session ID";

/// A failed result that only says the session to resume isn't there.
fn no_such_session(v: &Value) -> bool {
    let named = |e: &Value| e.as_str().is_some_and(|e| e.contains(NO_SUCH_SESSION));
    let turns = v.get("num_turns").and_then(Value::as_u64).unwrap_or(0);
    turns == 0 && (v.get("errors").and_then(Value::as_array).is_some_and(|a| a.iter().any(named)) || v.get("result").is_some_and(named))
}

/// Claude Code in headless mode (flags checked against `claude --help`).
#[derive(Default)]
pub struct ClaudeCode {
    /// The folder chosen for it ([`BackendOptions::config_dir`]); `None`: its usual one.
    folder: Option<std::path::PathBuf>,
}

impl ClaudeCode {
    /// The folder it keeps its sign-in and sessions in: the chosen one, else its usual one.
    fn config_dir(&self) -> Option<std::path::PathBuf> {
        self.folder.clone().or_else(claude_config_dir)
    }
}

impl AgentAdapter for ClaudeCode {
    fn ids(&self) -> &'static [&'static str] {
        &["claude-code", "claude"]
    }

    fn answers_questions(&self) -> bool {
        true
    }

    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    fn check_remote_tool_server(&self, host: &str, stop: &CancelToken) -> Result<(), ToolError> {
        remote_check(host, Some(stop))
    }

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(options.and_then(|options| options.cli_model.clone()).unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.into()))
    }

    fn model_choices(&self, _config: &BackendConfig, _options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(crate::anthropic::MODELS.iter().map(|model| model.to_string()).collect())
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        if cx.question {
            return question_args(a, cx);
        }
        let mcp_json = json!({"mcpServers": {MCP_NAME: {"command": cx.mcp.command, "args": cx.mcp.args}}}).to_string();
        a.extend(["-p".into(), "--output-format".into(), "stream-json".into(), "--verbose".into(), "--append-system-prompt".into(), cx.preamble.to_string()]);
        // Opus 5.5 everywhere unless config.toml names another model on purpose; the effort
        // goes in through the environment (see `run_cli`).
        a.extend(["--model".into(), cx.options.cli_model.clone().unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.into())]);
        if let Some(r) = cx.resume {
            a.extend(["--resume".into(), r.to_string()]);
        }
        // Deep builders keep subagents (#1105): a subagent has only these tools and the session's
        // permissions. Clean room leaves them out, since its reads must all reach our approval tool.
        let tools = match (cx.builder, cx.reads_checked) {
            (true, false) => "Bash,PowerShell,Read,Write,Edit,MultiEdit,Glob,Grep,TodoWrite,NotebookEdit,Agent",
            (true, true) => "Bash,PowerShell,Read,Write,Edit,MultiEdit,Glob,Grep,TodoWrite,NotebookEdit",
            (false, _) => "",
        };
        a.extend(["--tools".into(), tools.into()]);
        if cx.builder {
            // Questions go through our core_person_ask tool, which the app shows as buttons.
            a.extend(["--permission-prompt-tool".into(), format!("mcp__{MCP_NAME}__{APPROVAL_TOOL}"), "--disallowedTools".into(), "AskUserQuestion".into()]);
        }
        if let Some(dir) = cx.mcp.notes.as_deref().filter(|_| cx.builder) {
            // After every tool call, notes the person added meanwhile reach the model as context.
            a.extend(["--settings".into(), notes_hook_settings(&cx.mcp.command, dir)]);
        }
        a.extend(["--strict-mcp-config".into(), "--mcp-config".into(), mcp_json, "--allowedTools".into(), format!("mcp__{MCP_NAME}__*")]);
        if cx.builder && cx.reads_checked {
            a.push("TodoWrite".into());
        } else if cx.builder {
            a.extend(["Read", "Glob", "Grep", "TodoWrite"].map(String::from));
        } else if cx.reads_checked {
            // A Clean room chat works only through our tools, which keep the original out of reach.
            // Refused outright: neither its defaults nor an allow rule added later can open them (#564).
            a.push("--disallowedTools".into());
            a.extend(READING_TOOLS.iter().chain(CHANGING_TOOLS).map(|t| t.to_string()));
        }
        a
    }

    /// Claude Code's stream-json: text and tool calls as they come, the session id from any line,
    /// and from the final `result` its usage and cost, a failure in plain words, or a session to
    /// resume that isn't on this computer (`session_missing`, with its own message).
    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(id) = v.get("session_id").and_then(Value::as_str) {
            p.session_id = Some(id.to_string());
        }
        match s("type").as_str() {
            "system" if s("subtype") == "init" => {
                let failed = v.get("mcp_servers").and_then(Value::as_array).and_then(|servers| {
                    servers.iter().find(|server| {
                        server.get("name").and_then(Value::as_str) == Some(MCP_NAME)
                            && matches!(server.get("status").and_then(Value::as_str), Some("failed" | "needs-auth"))
                    })
                });
                if let Some(server) = failed {
                    p.failure = Some(TurnFailure::new(FailureKind::ToolsUnavailable));
                    let status = server.get("status").and_then(Value::as_str).unwrap_or_default();
                    out.push(AgentEvent::Error {
                        message: format!("Craft Anything's tools did not start in Claude Code (server status: {status})."),
                        remediation: Some(
                            "Check the Craft Anything CLI and the agent failure log, then start a new build attempt after fixing the tool connection.".into(),
                        ),
                        log: None,
                    });
                }
            }
            "assistant" => {
                if let Some(code) = v.get("error").and_then(Value::as_str) {
                    let failure = TurnFailure::claude(code);
                    p.failure = Some(failure);
                    let why = v.pointer("/message/content").map(content_text).unwrap_or_else(|| code.to_string());
                    let (message, hint) = claude_code_failure_in(&why, self.folder.as_deref());
                    let remediation = hint.or_else(|| match failure.kind {
                        FailureKind::Authentication => self.sign_in_fix(),
                        FailureKind::Billing => Some("Check the account's credits, plan and spending limit before trying again.".into()),
                        FailureKind::Configuration => Some("Check the chosen model and Claude Code settings before trying again.".into()),
                        _ => None,
                    });
                    return vec![AgentEvent::Error { message, remediation, log: None }];
                }
                for b in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    match b.get("type").and_then(Value::as_str).unwrap_or_default() {
                        "text" => out.push(AgentEvent::TextDelta { text: b.get("text").and_then(Value::as_str).unwrap_or_default().to_string() }),
                        "tool_use" => {
                            let id = b.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                            let name = b.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            out.push(p.started(&id, &name, b.get("input").cloned().unwrap_or(Value::Null)));
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for b in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_string();
                        let err = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let text = content_text(b.get("content").unwrap_or(&Value::Null));
                        out.push(p.finished(&id, !err, &text));
                    }
                }
            }
            "result" => {
                p.saw_result = true;
                p.cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
                if let Some(u) = v.get("usage") {
                    p.input_tokens = u
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .saturating_add(u.get("cache_read_input_tokens").and_then(Value::as_u64).unwrap_or(0));
                    p.output_tokens = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
                if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) && no_such_session(v) {
                    p.session_missing = true;
                    out.push(AgentEvent::Error {
                        message: "Claude Code's earlier conversation about this project isn't on this computer.".into(),
                        remediation: Some(
                            "It was started on another computer or in another folder. Ask again: a new conversation starts from what the project already has."
                                .into(),
                        ),
                        log: None,
                    });
                } else if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                    p.failure.get_or_insert(TurnFailure::new(FailureKind::Unknown));
                    let why = if s("result").is_empty() { s("subtype") } else { s("result") };
                    let (message, remediation) = claude_code_failure_in(&why, self.folder.as_deref());
                    out.push(AgentEvent::Error { message, remediation, log: None });
                }
            }
            _ => {}
        }
        out
    }

    fn resumes(&self) -> bool {
        true
    }

    /// The person's own standalone Claude Code (see [`host_session_vars`]), in its chosen folder.
    /// Under a Flatpak without one, it gets a folder it can write its conversations to (#506).
    fn process(&self, program: &str) -> Process {
        let proc = standalone(program);
        if let Some(dir) = &self.folder {
            return proc.env("CLAUDE_CONFIG_DIR", dir);
        }
        match flatpak_claude_dir() {
            Some(dir) => {
                let _ = std::fs::create_dir_all(&dir);
                proc.env("CLAUDE_CONFIG_DIR", dir)
            }
            None => proc,
        }
    }

    fn signed_in(&self, program: &str) -> Option<bool> {
        let out = match self.process(program).args(["auth", "status", "--json"]).timeout(Duration::from_secs(15)).stdout_cap(64 * 1024).run() {
            Ok(out) => out,
            Err(e) => {
                log::debug!("claude auth status --json didn't run: {e}");
                return None;
            }
        };
        // The status is the first JSON object printed: a certificate warning or a banner before it
        // isn't a reason to call the person signed out (#1830).
        let text = out.stdout_text();
        let Some(json) = first_json_object(&text) else {
            log::debug!("claude auth status --json: exit {:?}, {} bytes on stdout, no JSON object in it", out.code, text.len());
            return None;
        };
        let v: Value = match serde_json::from_str(json) {
            Ok(v) => v,
            Err(e) => {
                // The answer's wording, never its content: it names an account and a token mode.
                log::debug!("claude auth status --json: exit {:?}, {} bytes on stdout, its JSON didn't parse: {e}", out.code, text.len());
                return None;
            }
        };
        let signed = v.get("loggedIn").and_then(Value::as_bool);
        if signed != Some(true) {
            log::debug!("claude auth status --json: exit {:?}, {} bytes on stdout, loggedIn: {signed:?}", out.code, text.len());
        }
        signed
    }

    /// The account its `.claude.json` names (`oauthAccount`), kept there while the sign-in itself
    /// is in the system's keychain. In a chosen folder the file is inside it; with its usual
    /// `~/.claude`, it's beside it in the home folder.
    fn signed_in_before(&self) -> bool {
        let set = self.folder.clone().or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from));
        let file = match set.or_else(flatpak_claude_dir) {
            Some(dir) => dir.join(".claude.json"),
            None => match dirs::home_dir() {
                Some(home) => home.join(".claude.json"),
                None => return false,
            },
        };
        claude_names_account(&file)
    }

    fn sign_in_fix(&self) -> Option<String> {
        sign_in_hint_in("login", in_flatpak(), self.folder.as_deref())
    }

    /// The session under Claude Code's own folder (see [`claude_session_file`]).
    fn session_file(&self, id: &str) -> Option<std::path::PathBuf> {
        claude_session_file(&self.config_dir()?, id)
    }

    /// The kept session's requests and replies (see [`claude_handoff`]).
    fn handoff(&self, saved: &str, max: usize) -> Option<String> {
        claude_handoff(saved, max)
    }

    /// Back under Claude Code's own folder (see [`claude_restore`]); without one there's nowhere
    /// to put it, which is said.
    fn restore_session(&self, id: &str, saved: &[u8], cwd: &std::path::Path) -> Result<bool, ToolError> {
        let config = self.config_dir().ok_or_else(|| ToolError::new(ErrorCode::BadSettings, "Couldn't find Claude Code's folder on this computer."))?;
        claude_restore(&config, id, saved, cwd)
    }

    fn folder_var(&self) -> Option<&'static str> {
        Some("CLAUDE_CONFIG_DIR")
    }

    fn in_folder(&self, folder: std::path::PathBuf) -> Option<Arc<dyn AgentAdapter>> {
        Some(Arc::new(ClaudeCode { folder: Some(folder) }))
    }
}

/// Claude Code's own folder: `CLAUDE_CONFIG_DIR` when it's set (the person's choice, which the
/// agent we start keeps too), else our own under a Flatpak (see [`flatpak_claude_dir`]), else
/// `~/.claude`.
fn claude_config_dir() -> Option<std::path::PathBuf> {
    let set = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    set.or_else(flatpak_claude_dir).or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

pub use craft_anything_platform::in_flatpak;

/// Where Claude Code keeps its conversations when we start it under a Flatpak and the person
/// hasn't chosen a folder: its own is `~/.claude`, which the sandbox only lets us read, so a
/// conversation was never saved and `--resume` found nothing (#506). A folder of ours in the
/// app's data folder is writable, and kept with the app. `None` outside a Flatpak, or when
/// `CLAUDE_CONFIG_DIR` is set.
pub fn flatpak_claude_dir() -> Option<std::path::PathBuf> {
    let chosen = std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty());
    flatpak_claude_dir_in(in_flatpak(), chosen, dirs::data_dir())
}

pub fn flatpak_claude_dir_in(flatpak: bool, chosen: bool, data: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    (flatpak && !chosen).then_some(data?.join("craft-anything").join("claude"))
}

/// Claude Code's name for the folder a session ran in: the path with everything but letters and
/// digits made `-`. It finds a session by its id in any of these folders (2.1), so a long path
/// cut short still works.
fn claude_project_folder(cwd: &std::path::Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(200).collect()
}

/// Put a kept Claude Code session back under `config` (see [`AgentAdapter::restore_session`]).
/// Only a copy of that same session is put back, and never over one that's there.
pub fn claude_restore(config: &std::path::Path, id: &str, saved: &[u8], cwd: &std::path::Path) -> Result<bool, ToolError> {
    let valid = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(ToolError::invalid("The project names its builder's session with an id Claude Code doesn't use."));
    }
    if claude_session_file(config, id).is_some() {
        return Ok(false);
    }
    let named = format!("\"sessionId\":\"{id}\"");
    if !saved.windows(named.len()).any(|w| w == named.as_bytes()) {
        return Err(ToolError::invalid("The conversation kept in the project is of another session."));
    }
    let path = config.join("projects").join(claude_project_folder(cwd)).join(format!("{id}.jsonl"));
    craft_anything_platform::fs::write_atomic_private(&path, saved)?;
    Ok(true)
}

/// Folders under Claude Code's `projects/` looked through for a session.
const SESSION_FOLDERS: usize = 10_000;

/// Claude Code keeps each session as `projects/<the folder it ran in>/<id>.jsonl` under its
/// config folder. An id is a UUID: anything else (a hand-edited manifest) names no file.
pub fn claude_session_file(config: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let name = format!("{id}.jsonl");
    std::fs::read_dir(config.join("projects")).ok()?.flatten().take(SESSION_FOLDERS).map(|e| e.path().join(&name)).find(|p| p.is_file())
}

/// Whether Claude Code's `.claude.json` names the account it was signed in to (`oauthAccount`).
/// It also keeps every project's history, so up to 32 MB is read; a bigger or unreadable file
/// names none.
pub fn claude_names_account(file: &std::path::Path) -> bool {
    use std::io::Read;
    const CAP: u64 = 32 << 20;
    let mut text = Vec::new();
    let read = std::fs::File::open(file).and_then(|f| f.take(CAP + 1).read_to_end(&mut text));
    if read.is_err() || text.len() as u64 > CAP {
        return false;
    }
    serde_json::from_slice::<Value>(&text).ok().is_some_and(|v| v.get("oauthAccount").is_some_and(Value::is_object))
}

/// Longest single request or reply in a handoff; a first brief runs to many thousands.
const HANDOFF_ENTRY: usize = 1500;

/// A Claude Code session read back as a handoff: the messages Craft Anything sent and the text
/// the builder answered with, oldest first, leaving out tool calls, their results, thinking and
/// side tasks. The newest are kept when it's longer than `max` characters.
pub fn claude_handoff(saved: &str, max: usize) -> Option<String> {
    let clip = |t: &str| {
        let t = t.trim();
        if t.chars().count() > HANDOFF_ENTRY { format!("{}…", t.chars().take(HANDOFF_ENTRY).collect::<String>()) } else { t.to_string() }
    };
    let mut entries = Vec::new();
    for v in saved.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let content = v.pointer("/message/content");
        match v.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(t) = content.and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                    entries.push(format!("Craft Anything asked you: {}", clip(t)));
                }
            }
            Some("assistant") => {
                let text: String = content
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() && text.trim() != "No response requested." {
                    entries.push(format!("You answered: {}", clip(&text)));
                }
            }
            _ => {}
        }
    }
    let mut kept = Vec::new();
    let mut used = 0usize;
    for e in entries.iter().rev() {
        let n = e.chars().count().saturating_add(2);
        if used.saturating_add(n) > max {
            break;
        }
        used = used.saturating_add(n);
        kept.push(e.as_str());
    }
    kept.reverse();
    (!kept.is_empty()).then(|| kept.join("\n\n"))
}

/// Codex (`codex exec --json`). Flags are checked against `codex exec --help` before use.
pub struct Codex;

impl AgentAdapter for Codex {
    fn ids(&self) -> &'static [&'static str] {
        &["codex"]
    }

    fn resumes(&self) -> bool {
        true
    }

    fn check_native_options(&self, options: &BackendOptions, _cwd: Option<&Path>) -> Result<(), ToolError> {
        if options.args.iter().any(|arg| arg == "--ephemeral") {
            return Err(ToolError::invalid("Codex's --ephemeral option does not save conversations for later turns.")
                .with_remediation("Remove --ephemeral from this backend's arguments so Craft Anything can resume the project's conversation."));
        }
        if options.args.iter().any(|arg| arg == "--worktree") {
            return Err(ToolError::invalid("Codex's --worktree option cannot continue this project's saved conversation.").with_remediation(
                "Remove --worktree from this backend's arguments. To work in a worktree, select its existing folder as the project folder.",
            ));
        }
        Ok(())
    }

    fn parse_stderr(&self, p: &mut StreamParser, line: &str) -> Vec<AgentEvent> {
        let diagnostic = line.trim().to_ascii_lowercase();
        if p.session_missing || !diagnostic.starts_with("error:") || !diagnostic.contains("no rollout found for thread id ") {
            return Vec::new();
        }
        p.session_missing = true;
        vec![AgentEvent::Error {
            message: "Codex's earlier conversation about this project isn't on this computer.".into(),
            remediation: Some("Ask again: a new conversation starts from what the project already has.".into()),
            log: None,
        }]
    }

    /// Ask the selected CLI, not its auth file or keychain; discard raw status output.
    fn signed_in(&self, program: &str) -> Option<bool> {
        let out = Process::new(program).args(["login", "status"]).timeout(Duration::from_secs(10)).stdout_cap(16 * 1024).run().ok()?;
        codex_login_status(out.success, &out.stdout_text(), &out.stderr_text())
    }

    fn sign_in_fix(&self) -> Option<String> {
        Some("Run the selected Codex program with `login` in a terminal, choose Sign in with ChatGPT, then test it again in Settings → AI.".into())
    }

    /// `codex exec -` reads the message from stdin, so a long one can't pass Windows'
    /// command-line limit (os error 206, #528).
    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn stdin_text(&self, preamble: &str, prompt: &str) -> String {
        format!("{preamble}{prompt}")
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    fn check_remote_tool_server(&self, host: &str, stop: &CancelToken) -> Result<(), ToolError> {
        remote_check(host, Some(stop))
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut v = vec!["exec".to_string(), "--json".into()];
        // Project folders aren't git repositories, and `codex exec` refuses to run outside one
        // without this (#69). Not twice: it may already be in the backend's own args.
        const SKIP_GIT: &str = "--skip-git-repo-check";
        if !cx.options.args.iter().any(|a| a == SKIP_GIT) {
            v.push(SKIP_GIT.into());
        }
        if cx.builder {
            v.extend(["--sandbox".into(), "workspace-write".into()]);
        }
        v.extend(["-c".into(), format!("mcp_servers.{MCP_NAME}.command={}", toml_string(&cx.mcp.command))]);
        // Codex doesn't refresh its callable tools after enable_toolset (#538), so list every
        // permitted tool from startup. The server still applies Clean room's restrictions.
        let mut mcp_args = cx.mcp.args.clone();
        if !mcp_args.iter().any(|a| a == "--all-tools") {
            mcp_args.push("--all-tools".into());
        }
        let args_toml = format!("[{}]", mcp_args.iter().map(|s| toml_string(s)).collect::<Vec<_>>().join(", "));
        v.extend(["-c".into(), format!("mcp_servers.{MCP_NAME}.args={args_toml}")]);
        // `codex exec` can't ask, so without this it refuses a tool not marked read-only, such as
        // rebuild_start, before our server sees the call (#540). Only tools that ask the person
        // themselves are approved; every other tool keeps the person's own Codex policy.
        for tool in craft_anything_engine::AGENT_APP_PREAPPROVED_TOOLS {
            v.extend(["-c".into(), format!("mcp_servers.{MCP_NAME}.tools.{tool}.approval_mode=\"approve\"")]);
        }
        if let Some(m) = &cx.options.cli_model {
            v.extend(["-m".into(), m.clone()]);
        }
        v.extend(cx.options.args.iter().cloned());
        // Exec-only flags (notably --sandbox) stay before the resume subcommand.
        // The saved thread id is positional data, never another CLI option.
        if let Some(id) = cx.resume {
            v.extend(["resume".into(), "--".into(), id.into()]);
        }
        // The message itself goes in on stdin (see `stdin_text`).
        v.push("-".into());
        v
    }

    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["exec", "--help"], &["--json"])
    }

    /// Plain `codex <prompt>` is the interactive TUI: "stdin is not a terminal".
    fn plain_fallback(&self) -> bool {
        false
    }

    /// Translate Codex JSON events; only explicit authentication failures get sign-in guidance.
    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let t = v.get("type").and_then(Value::as_str).unwrap_or_default();
        if let Some(id) = v.get("thread_id").and_then(Value::as_str) {
            p.session_id = Some(id.to_string());
        }
        let item = v.get("item").cloned().unwrap_or(Value::Null);
        let it = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        match (t, it) {
            ("item.completed", "agent_message") => {
                out.push(AgentEvent::TextDelta { text: format!("{}\n", item.get("text").and_then(Value::as_str).unwrap_or_default()) })
            }
            ("item.completed", "reasoning") => out.push(AgentEvent::Status { text: summary_of(item.get("text").and_then(Value::as_str).unwrap_or_default()) }),
            ("item.started", "mcp_tool_call") => {
                let name = item.get("tool").and_then(Value::as_str).unwrap_or_default().to_string();
                out.push(p.started(&id, &name, item.get("arguments").cloned().unwrap_or(Value::Null)));
            }
            ("item.completed", "mcp_tool_call") => {
                if !p.tool_names.contains_key(&id) {
                    let name = item.get("tool").and_then(Value::as_str).unwrap_or_default().to_string();
                    out.push(p.started(&id, &name, item.get("arguments").cloned().unwrap_or(Value::Null)));
                }
                let failed = item.get("status").and_then(Value::as_str) == Some("failed") || item.get("error").is_some_and(|e| !e.is_null());
                let tool = item.get("tool").and_then(Value::as_str).unwrap_or_default();
                let ours = item.get("server").and_then(Value::as_str) == Some(MCP_NAME);
                let text = codex_tool_text(&item, tool, ours);
                out.push(p.finished(&id, !failed, &text));
            }
            ("item.started", "command_execution") => {
                out.push(AgentEvent::Status { text: format!("Running: {}", summary_of(item.get("command").and_then(Value::as_str).unwrap_or_default())) })
            }
            ("turn.completed", _) => {
                p.saw_result = true;
                if let Some(u) = v.get("usage") {
                    p.input_tokens = p.input_tokens.saturating_add(u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0));
                    p.output_tokens = p.output_tokens.saturating_add(u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0));
                }
            }
            ("turn.failed", _) | ("error", _) => {
                let m = v.pointer("/error/message").or_else(|| v.get("message")).and_then(Value::as_str).unwrap_or("unknown error");
                let lower = m.to_ascii_lowercase();
                let auth = [
                    "unauthorized",
                    "not logged in",
                    "authentication expired",
                    "authentication failed",
                    "authentication required",
                    "login required",
                    "token expired",
                ]
                .iter()
                .any(|phrase| lower.contains(phrase));
                out.push(AgentEvent::Error { message: format!("Codex: {m}"), remediation: if auth { self.sign_in_fix() } else { None }, log: None });
            }
            _ => {}
        }
        out
    }
}

/// Unknown output (including future CLI formats) is not a claim that the account is connected.
fn codex_login_status(success: bool, stdout: &str, stderr: &str) -> Option<bool> {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    if text.contains("not logged in") {
        Some(false)
    } else if success && text.contains("logged in") {
        Some(true)
    } else {
        None
    }
}

/// What a finished Codex tool call says. A failed call carries `"result": null` beside its
/// `error`, which showed as "null" (#540), so null fields are skipped and an error's message is
/// used as written. Codex's refusal to run one of our tools it can't ask about gets the way out.
fn codex_tool_text(item: &Value, tool: &str, ours: bool) -> String {
    let field = |k: &str| item.get(k).filter(|v| !v.is_null());
    let error = field("error").map(|e| e.get("message").and_then(Value::as_str).map_or_else(|| e.to_string(), str::to_string));
    let text = match (field("result"), error) {
        (_, Some(e)) => e,
        (Some(r), None) => r.to_string(),
        (None, None) => String::new(),
    };
    // Another MCP server's refusal isn't fixed by our setup: it keeps Codex's own words.
    if ours && text.contains(CODEX_NEEDS_APPROVAL) {
        // The feed keeps 200 characters of a result, so the fix comes first and fits.
        return format!(
            "Codex blocked {tool}: it can't ask for approval here. Fix: run `craft-anything-cli setup --client codex --yes`, then retry. See docs/codex.md."
        );
    }
    text
}

/// Codex's wording when a tool needs approval and its policy is `never` (#540).
const CODEX_NEEDS_APPROVAL: &str = "requires approval, but approval policy is never";

/// Gemini CLI (`--output-format stream-json`). Flags are checked against `gemini --help`.
pub struct GeminiCli;

impl AgentAdapter for GeminiCli {
    fn ids(&self) -> &'static [&'static str] {
        &["gemini-cli", "gemini"]
    }

    /// Gemini CLI reads its message from stdin and appends `-p` to it, so a long one can't pass
    /// Windows' command-line limit, which is 8,191 characters through npm's `gemini.cmd`.
    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn stdin_text(&self, preamble: &str, prompt: &str) -> String {
        format!("{preamble}{prompt}")
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        if let Some(m) = &cx.options.cli_model {
            a.extend(["-m".into(), m.clone()]);
        }
        // An empty `-p` keeps it headless and adds nothing to the message on stdin (see `stdin_text`).
        a.extend(["--output-format".into(), "stream-json".into(), "-p".into(), String::new()]);
        a
    }

    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["--help"], &["--output-format", "stream-json"])
    }

    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match s("type").as_str() {
            "init" => p.session_id = v.get("session_id").and_then(Value::as_str).map(str::to_string),
            "message" if s("role") == "assistant" => out.push(AgentEvent::TextDelta { text: s("content") }),
            "tool_use" => {
                let id = s("tool_id");
                let name = s("tool_name");
                out.push(p.started(&id, &name, v.get("parameters").cloned().unwrap_or(Value::Null)));
            }
            "tool_result" => {
                let id = s("tool_id");
                let ok = s("status") != "error";
                let text = v.get("output").map(content_text).unwrap_or_default();
                out.push(p.finished(&id, ok, &text));
            }
            "error" => out.push(AgentEvent::Error { message: format!("Gemini CLI: {}", s("message")), remediation: None, log: None }),
            "result" => {
                p.saw_result = true;
                if let Some(st) = v.get("stats") {
                    p.input_tokens = st.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    p.output_tokens = st.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
                // Documented stream-json only ever ends a run with `result` on success (a failure
                // is its own `error` event instead), but a `status` other than "success" here, if
                // one is ever sent, must still fail the turn rather than quietly counting as done:
                // `saw_result` alone now also decides whether a lingering process is treated as
                // finished (`RESULT_GRACE` in `run_cli`).
                if let Some(status) = v.get("status").and_then(Value::as_str).filter(|s| *s != "success") {
                    let why = v.get("error").and_then(Value::as_str).unwrap_or(status);
                    out.push(AgentEvent::Error { message: format!("Gemini CLI: {why}"), remediation: None, log: None });
                }
            }
            _ => {}
        }
        out
    }
}

/// One `ssh` run for a remote set-up step. Never interactive (`BatchMode`, stdin closed).
fn ssh_run(host: &str, line: &str, timeout: std::time::Duration, stop: Option<&CancelToken>) -> Result<craft_anything_platform::Output, ToolError> {
    let argv = crate::ssh::ssh_line_args(host, line.to_string());
    let mut proc = craft_anything_platform::process::Process::new("ssh").args(&argv).timeout(timeout).stdout_cap(64 * 1024);
    if let Some(stop) = stop {
        let stop = stop.clone();
        proc = proc.cancel_when(move || stop.is_cancelled());
    }
    proc.run()
}

/// `command -v` only promises a nonzero status for a missing program: 127 is the shell's own "not
/// found", which [`crate::ssh::diagnose`] turns into how to install it or put it on the PATH.
/// It prints where the helper is, which [`remote_cli_path`] reads.
pub(super) const REMOTE_CHECK_LINE: &str = "command -v craft-anything-cli || exit 127";

/// Where `craft-anything-cli` is on `host`, as an absolute path. Claude Code starts the tool server
/// with its own PATH, not the one ssh gave this check: by bare name the server failed on a host
/// where the check passed ("server status: failed"), and by absolute path it connected. None: the
/// host didn't say, so the bare name stays and the build's own error explains.
pub(super) fn remote_cli_path(host: &str) -> Option<String> {
    let out = ssh_run(host, REMOTE_CHECK_LINE, std::time::Duration::from_secs(15), None).ok().filter(|out| out.success)?;
    absolute_path_line(&out.stdout_text())
}

/// The first JSON object in `text`, to its closing brace. A CLI can print a warning (a certificate
/// notice on Windows, a banner) before its answer, and the answer is still the object (#1830).
fn first_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let rest = text.get(start..)?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return rest.get(..i + c.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// The first line of `text` when it is an absolute path; a shell function, alias or banner isn't.
fn absolute_path_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    line.starts_with('/').then(|| line.to_string())
}

#[cfg(test)]
mod first_json_object_tests {
    use super::first_json_object;

    /// #1830: a warning printed before the status doesn't hide it, and braces inside text don't
    /// end it early.
    #[test]
    fn the_first_json_object_is_taken_from_noisy_output() {
        assert_eq!(first_json_object("{\"loggedIn\":true}"), Some("{\"loggedIn\":true}"));
        let noisy = "(node:1) Warning: self-signed certificate\n{\n  \"loggedIn\": true,\n  \"note\": \"a } inside text\"\n}\n";
        let json = first_json_object(noisy).expect("the object after the warning");
        let parsed: serde_json::Value = serde_json::from_str(json).expect("balanced at the right brace");
        assert_eq!(parsed["loggedIn"], true);
        assert_eq!(first_json_object("nothing here"), None);
        assert_eq!(first_json_object("{\"unfinished\": "), None);
    }
}

#[cfg(test)]
mod remote_path_tests {
    use super::absolute_path_line;

    #[test]
    fn only_an_absolute_path_is_taken_as_the_helper_location() {
        assert_eq!(absolute_path_line("/home/a/.cargo/bin/craft-anything-cli\n").as_deref(), Some("/home/a/.cargo/bin/craft-anything-cli"));
        assert_eq!(absolute_path_line("\n  /opt/ca/craft-anything-cli  \n").as_deref(), Some("/opt/ca/craft-anything-cli"));
        assert_eq!(absolute_path_line("craft-anything-cli"), None);
        assert_eq!(absolute_path_line("alias craft-anything-cli='x'"), None);
        assert_eq!(absolute_path_line(""), None);
    }
}

/// The remote helper is on PATH, checked before a build starts (the remote server runs it). For
/// every agent that's handed our tool server over SSH, as the local check is for one here.
fn remote_check(host: &str, stop: Option<&CancelToken>) -> Result<(), ToolError> {
    let out = ssh_run(host, REMOTE_CHECK_LINE, std::time::Duration::from_secs(15), stop)?;
    if out.success {
        return Ok(());
    }
    Err(remote_failure(host, &out))
}

/// Map the remote failure onto the fix, reusing the ssh failure wording other remote runs use.
pub(super) fn remote_failure(host: &str, out: &craft_anything_platform::Output) -> ToolError {
    if let Some(failure) = crate::ssh::diagnose(host, "craft-anything-cli", crate::ssh::is_git_server(host), out.code, out.stderr_end_text().as_str()) {
        return ToolError::new(ErrorCode::EngineMissing, format!("Craft Anything can't set up its tools on {host}: {}.", failure.what))
            .with_remediation(failure.fix);
    }
    let end = out.end_text();
    let trimmed = end.trim();
    let n = trimmed.chars().count();
    let snippet: String = trimmed.chars().skip(n.saturating_sub(200)).collect();
    let exit = out.code.map_or_else(|| "it was stopped".to_string(), |code| format!("exit {code}"));
    ToolError::new(ErrorCode::EngineMissing, format!("Craft Anything couldn't set up its tools on {host}: ssh didn't come back as expected ({exit})."))
        .with_remediation(if snippet.is_empty() {
            "Check that craft-anything-cli is on the PATH this ssh session uses, and that the host lets you run programs over ssh.".to_string()
        } else {
            format!("ssh said: {snippet}")
        })
}

#[cfg(test)]
mod codex_login_tests {
    use super::codex_login_status;

    #[test]
    fn login_status_reads_both_streams_without_inferring_success_from_exit_alone() {
        assert_eq!(codex_login_status(true, "", "Logged in using ChatGPT"), Some(true));
        assert_eq!(codex_login_status(true, "Logged in using an API key", ""), Some(true));
        assert_eq!(codex_login_status(false, "", "Not logged in"), Some(false));
        assert_eq!(codex_login_status(true, "Not logged in", ""), Some(false));
        assert_eq!(codex_login_status(false, "", "could not read credentials"), None);
        assert_eq!(codex_login_status(true, "", ""), None);
    }
}
