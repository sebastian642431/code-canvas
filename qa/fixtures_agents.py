import datetime
import json
import os


def stamp(seconds_ago):
    when = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=seconds_ago)
    return when.strftime("%Y-%m-%dT%H:%M:%S.") + f"{when.microsecond // 1000:03d}Z"


def codex_home(root):
    """What Codex keeps in ~/.codex: one session that was at work a few minutes
    before the app started, in the shape its session files have (one JSON object
    a line). Its folder is the `rust-small` fixture beside this one."""
    project = os.path.join(os.path.dirname(root), "rust-small").replace("\\", "/")
    session = "01a1ec52-0000-7000-8000-000000000001"
    lines = [
        (300, "session_meta", {"id": session, "cwd": project, "originator": "codex_cli_rs",
                               "cli_version": "0.169.0", "source": "cli", "base_instructions": {"text": "..."}}),
        (300, "event_msg", {"type": "task_started", "turn_id": "t1"}),
        (299, "turn_context", {"cwd": project, "approval_policy": "on-request", "model": "gpt-5"}),
        (299, "response_item", {"type": "message", "role": "user", "content": [
            {"type": "input_text", "text": "<environment_context>\n  <cwd>" + project + "</cwd>\n</environment_context>"}]}),
        (298, "event_msg", {"type": "item_completed", "thread_id": session, "turn_id": "t1", "item": {
            "type": "UserMessage", "id": "item-1", "content": [{"type": "text", "text": "Explain how the steps are counted"}]}}),
        (240, "event_msg", {"type": "item_completed", "thread_id": session, "turn_id": "t1", "item": {
            "type": "CommandExecution", "id": "exec-1", "command": ["bash", "-lc", "cat src/util.rs"],
            "cwd": project, "parsed_cmd": [{"type": "read", "cmd": "cat src/util.rs", "name": "util.rs", "path": "src/util.rs"}],
            "status": "completed", "exit_code": 0}}),
        (120, "response_item", {"type": "function_call", "name": "shell_command", "call_id": "call-2",
                                "arguments": json.dumps({"command": "sed -n '1,7p' src/main.rs", "workdir": project})}),
    ]
    rel = f"sessions/2026/01/01/rollout-2026-01-01T10-00-00-{session}.jsonl"
    path = os.path.join(root, *rel.split("/"))
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf8", newline="\n") as out:
        for seconds_ago, kind, payload in lines:
            line = {"timestamp": stamp(seconds_ago), "type": kind, "payload": payload}
            out.write(json.dumps(line, separators=(",", ":")) + "\n")


def claude_transcripts(root):
    """Transcripts as Claude Code writes them: a session `s1.jsonl` whose answers
    come from Opus (one older answer from Sonnet before it), and its helper `a7`
    in `s1/subagents/agent-a7.jsonl`, answering with Haiku. Hook events of a
    scenario point at them with `transcript_path`."""
    def entry(kind, model=None, text="..."):
        message = {"role": kind, "content": [{"type": "text", "text": text}]}
        if model:
            message["model"] = model
        return {"type": kind, "sessionId": "s1", "message": message}

    def write(rel, entries):
        path = os.path.join(root, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf8", newline="\n") as out:
            for line in entries:
                out.write(json.dumps(line, separators=(",", ":")) + "\n")

    write("s1.jsonl", [
        entry("user", text="Make the steps faster"),
        entry("assistant", "claude-sonnet-5-5"),
        entry("user", text="and keep the tests"),
        entry("assistant", "claude-opus-5-5"),
        {"type": "user", "toolUseResult": "x" * 4000},
    ])
    write("s1/subagents/agent-a7.jsonl", [
        entry("user", text="Find where the steps are counted"),
        entry("assistant", "claude-haiku-5-5"),
    ])


FIXTURES = {
    "codex-home": codex_home,
    "claude-transcripts": claude_transcripts,
}
