//! Opt-in live check: uses the installed, signed-in Codex and its model allowance.
//! Build craft-anything-cli first and put target/debug on PATH. Never run in CI.

use craft_anything_ai::cli_agents::{Builder, CliConversation, CliSpec};
use craft_anything_ai::{AgentEvent, BackendConfig, BackendKind, BackendOptions, Conversation};
use std::error::Error;
use std::time::{Duration, Instant};

fn turn(c: &mut CliConversation, prompt: &str) -> Result<(String, bool), Box<dyn Error>> {
    c.send(prompt);
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut text = String::new();
    let mut listed_targets = false;
    let mut target_call = None;
    let mut error = None;
    while Instant::now() < deadline {
        for event in c.poll() {
            match event {
                AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
                AgentEvent::ToolStarted { call_id, tool, .. } => {
                    eprintln!("Tool: {tool}");
                    if tool == "core_targets_list" {
                        target_call = Some(call_id);
                    }
                }
                AgentEvent::ToolFinished { call_id, ok, .. } => {
                    if !ok {
                        c.stop();
                        return Err("MCP tool failed".into());
                    }
                    listed_targets |= target_call.as_deref() == Some(call_id.as_str());
                }
                AgentEvent::Error { message, .. } => {
                    error = Some(message);
                }
                AgentEvent::TurnDone { .. } => {
                    return match error {
                        Some(message) => Err(message.into()),
                        None => Ok((text.trim().to_string(), listed_targets)),
                    };
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    c.stop();
    Err("Codex smoke check exceeded three minutes".into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let config = BackendConfig { name: "Codex".into(), kind: BackendKind::AgentCli, model: "codex".into(), base_url: None, ssh_host: None, enabled: true };
    let options = BackendOptions {
        cli_model: std::env::args().nth(1),
        // Isolate this manual check from the person's other MCP servers and plugins.
        args: vec!["--ignore-user-config".into()],
        ..Default::default()
    };
    let mut spec = CliSpec::new(config, options, false, Some(dir.path().to_path_buf()));
    spec.builder = Some(Builder { investigation: None, inbox: dir.path().join("inbox") });
    spec.preamble = Some("This is a bounded adapter smoke check. Only perform the requested actions in the working folder. ".into());
    let mut c = CliConversation::new(spec.clone());
    if std::env::args().nth(2).as_deref() == Some("--missing-thread") {
        let mut missing = spec.clone();
        missing.resume = Some("00000000-0000-0000-0000-000000000000".into());
        c = CliConversation::new(missing);
        if turn(&mut c, "Reply READY without tools.").is_ok() || !c.session_lost() || c.session_id().is_some() {
            return Err("An unknown thread was not reported and cleared".into());
        }
        println!("PASS: unknown thread reported and cleared; testing a fresh turn next.");
    }
    let (reply, tool_ok) = turn(
        &mut c,
        "Remember the marker amber-sail-47 in this conversation. Call Craft Anything's core_targets_list MCP tool. Create smoke.txt containing exactly amber-sail-47 in the working folder. Do not use any other tools except those needed to create the file. Reply exactly READY.",
    )?;
    if reply.lines().last() != Some("READY") || !tool_ok || std::fs::read_to_string(dir.path().join("smoke.txt"))?.trim() != "amber-sail-47" {
        return Err("First turn did not verify MCP, file creation and the expected reply".into());
    }
    let saved = c.session_id().ok_or("Codex did not report a thread id")?;
    let follow_up = "What marker did I ask you to remember? Use conversation memory only, without tools or reading files. Reply with only the marker.";
    let (reply, _) = turn(&mut c, follow_up)?;
    if reply != "amber-sail-47" || c.session_id().as_deref() != Some(saved.as_str()) {
        return Err("The second turn did not retain the original thread and marker".into());
    }
    spec.resume = Some(saved.clone());
    drop(c);
    let mut reopened = CliConversation::new(spec);
    let (reply, _) = turn(&mut reopened, follow_up)?;
    if reply != "amber-sail-47" || reopened.session_id().as_deref() != Some(saved.as_str()) {
        return Err("A reopened conversation did not retain the original thread and marker".into());
    }
    println!("PASS: live MCP call, sandboxed file creation, second-turn resume and saved-thread resume.");
    Ok(())
}
