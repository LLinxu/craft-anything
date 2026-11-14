# Using Codex with Craft Anything

Craft Anything reaches Codex in two ways, and both have to get past Codex's own tool approval.

## Find and check Codex on macOS

An app opened from the Dock does not inherit the terminal's PATH. When PATH has
no Codex, detection and execution also look for its CLI in known ChatGPT.app and
Codex.app layouts, first under the user's Applications folder and then the system
Applications folder. Only executable files are accepted; the desktop GUI program
is never used as a coding agent. An explicitly selected program and PATH remain
ahead of the bundle fallback. A saved bare `codex` command gets the same fallback.
There is no shell-profile edit, global symlink, recursive scan or app launch.

In Settings → AI, add Codex under Found on this computer → Harnesses, then use
**Check** in its configured row. The check runs the selected program's `--version`
and `login status` on a worker. `craft-anything-cli ai test <name>` uses the same
headless API. It sends no prompt, and its `version_only` flag remains true:
installation and sign-in do not prove model entitlement or remaining usage.
Unknown or failed status output leaves the version reply unchanged, without a
signed-in claim. Every configured local agent CLI has the same Check action;
its adapter determines whether sign-in can be verified, using that backend's
selected configuration folder where supported.

Codex owns authentication. Run the selected Codex program with `login` in a
terminal and choose Sign in with ChatGPT, or use Codex's supported API-key login.
Craft Anything does not read, copy or store Codex's credential files or tokens.
This is not native ChatGPT OAuth inside Craft Anything and does not change
Codex's sandbox, approval policy or model selection. A known signed-out state
and recognized authentication failures include sign-in guidance. Unrelated
errors such as a failed login shell do not get authentication advice. SSH connection
tests retain their existing behavior; the Settings action is local-only.

The bundle layouts, executable permissions, root order, login-status parsing,
selected-CLI connection check and Settings result have isolated regressions.
Fixtures use fake programs and temporary directories, never real credentials.
Windows/Linux live-account behavior and model availability are not established
by these tests.

## Why a tool can be refused

`codex exec` is non-interactive: it has no one to ask, so its approval policy is `never`. A tool
that an MCP server doesn't mark read-only (such as `rebuild_start`, which runs a coding agent) is
refused before our server sees the call, with `requires approval, but approval policy is never`.
Codex's own settings decide this; Craft Anything's consent is a second, separate check.

## What we set

Only tools that ask the person themselves are approved on Codex's side
(`AGENT_APP_PREAPPROVED_TOOLS` in the engine, today just `rebuild_start`; a test checks each one
asks for consent). Every other tool keeps whatever approval policy you set in Codex.

- **Connecting Codex to Craft Anything** (`craft-anything-cli setup --client codex --yes`, or the
  Connect screen) writes the entry in `~/.codex/config.toml` (`$CODEX_HOME/config.toml` if set):

  ```toml
  [mcp_servers.craft-anything]
  command = "…/craft-anything-cli"
  args = ["mcp"]
  startup_timeout_sec = 30
  tools = { rebuild_start = { approval_mode = "approve" } }
  ```

  Your other entries and comments stay as they are, the old file is kept as
  `config.toml.craft-anything.backup`, and settings you added to our entry (`env`,
  `enabled_tools`, approvals for other tools) are kept when setup updates it. Setting
  `approval_mode` for `rebuild_start` yourself is overwritten by the next setup run.
  `craft-anything-cli doctor --client codex` reports an entry without the approval as "a different
  copy or shape of the command"; running setup again fixes it.
- **Building with Codex from the app** starts `codex exec` with the same setting on its `-c`
  flags, so nothing in your config is needed. Chats and builds also start the Craft Anything MCP
  server with `--all-tools`: Codex doesn't refresh its callable tool list after `enable_toolset`,
  so every tool permitted by the current mode is listed from startup. Clean room restrictions still apply.

## Who still asks

Approving `rebuild_start` in Codex doesn't remove Craft Anything's own consent: it runs a coding
agent, so our server asks before it runs. Who can answer depends on how the server was started:

- **Builds started from the app** run the server with the project's inbox, so the person answers
  in the app (Allow / Don't allow).
- **The entry from `setup`** starts `craft-anything-cli mcp` on its own, with no app attached, so
  there is no approval card: the call returns `consent_required` and nothing runs. To let Codex
  start builds this way, add `"--allow-run"` to the entry's `args` (every tool that runs code is
  then treated as allowed, so only do it for a Codex you trust), or start the build from the app.

In Clean room mode the tools that touch the original stay refused either way.

## If it still fails

A refusal for one of our tools now says what was blocked and how to fix it, instead of showing
`null`; another MCP server's refusal keeps Codex's own words, because our setup can't change
that server's policy. To approve more of our tools by hand, add
`[mcp_servers.craft-anything.tools.<tool>]` with `approval_mode = "approve"` and start the chat
again. Approving all of them (`default_tools_approval_mode = "approve"`) also approves tools with
no consent check of their own, such as `project_inbox_answer`, which can answer a pending approval,
so we don't do it.

Startup tool discovery was verified with Codex CLI 0.166.0, a loopback model and the real MCP server:
Codex received 107 Clean-room-permitted tools, including non-Core tools, while Deep-only tools stayed
hidden. No real-account build was run.

## Continuing a Codex session

The adapter saves Codex's reported thread id and uses `codex exec resume <id>` for
later turns, including a project reopened with its saved agent session. The run's
model, sandbox and MCP configuration are passed again. Exec-only flags stay before
the `resume` subcommand, the id is protected as positional data, and the prompt
continues to arrive on stdin. It never selects the most recent unrelated session.

Argument regressions and a local stand-in program cover a fresh turn, a second
turn and reopening a saved conversation. To explicitly check an installed Codex
against a real account, build the CLI and run the manual example:

```sh
cargo build -p craft-anything-cli
PATH="$PWD/target/debug:$PATH" cargo run -p craft-anything-ai --example codex_resume_smoke -- <model>
```

The model argument is optional. The example uses the installed Codex's sign-in
and model allowance; it is not part of automated tests. It ignores Codex's user
configuration for this check, calls the real `core_targets_list` MCP tool, creates
one file in a temporary workspace using the workspace-write sandbox, then verifies
both an in-memory follow-up and a reopened conversation keep the thread and marker.
The temporary workspace is removed afterwards; Codex keeps its usual session record.
It does not test a complete Study → Build → Check → Ready project or GUI behavior.

This check passed on macOS with Codex CLI 0.169.0-alpha.11.1 and `gpt-6.1-sol`
on 2026-11-14, using an existing ChatGPT sign-in.

Resume argument order (including `--sandbox` before `resume`) has been live-checked
only on Codex 0.169.0-alpha.11.1. Older releases, including 0.167, have not been
verified with this adapter; passing `--sandbox` after `resume` is not supported by
the checked CLI. The overlapping PR #1805 reports a config-override approach on 0.167.

A missing saved thread is reported on stderr as `no rollout found for thread id`.
The adapter marks the session missing, clears the saved id and explains that asking
again starts a fresh conversation. Other stderr failures do not clear the session.
To include this recovery in the manual live check, append `--missing-thread` after
the model argument. It first tries a deliberately nonexistent id and then runs
the normal fresh-turn, follow-up and reopened-session checks.
This extended check passed with 0.169.0-alpha.11.1 on macOS on 2026-11-14.

The native adapter refuses a backend's `--worktree` option before starting any turn,
because Codex 0.169 rejects it with resume. Select an existing worktree's folder as
the project folder instead.

The native adapter also refuses `--ephemeral` before starting a turn: Codex does
not persist those conversations, so their reported ids cannot be resumed later.
Remove that option to use the project's saved conversation. A two-turn subprocess
regression checks that both attempts explain the refusal, start no Codex process
and save no session id.
