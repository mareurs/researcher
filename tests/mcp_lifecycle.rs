#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct McpChild {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: mpsc::Receiver<String>,
}

impl McpChild {
    fn start() -> Self {
        let cwd = tempfile::tempdir().expect("isolated config directory");
        let mut child = Command::new(env!("CARGO_BIN_EXE_researcher-mcp"))
            .current_dir(cwd.path())
            .env("MCP_IDLE_TIMEOUT_SECS", "0")
            .env("LLM_PROVIDER", "vertex")
            .env("LLM_FAST_PROVIDER", "vertex")
            .env("VERTEX_PROJECT", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start MCP server");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            stdin: Some(stdin),
            responses: rx,
        };
        let response = server.request(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "lifecycle-test", "version": "1"}
            }
        }));
        assert!(response.get("result").is_some(), "{response}");
        let stdin = server.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            })
        )
        .expect("send initialized");
        stdin.flush().expect("flush initialized");
        server
    }
    fn request(&mut self, request: serde_json::Value) -> serde_json::Value {
        let stdin = self.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{request}").expect("send request");
        stdin.flush().expect("flush request");
        let response = self
            .responses
            .recv_timeout(Duration::from_secs(10))
            .expect("MCP response");
        serde_json::from_str(&response).expect("JSON response")
    }

    fn assert_alive_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            assert_eq!(
                self.child.try_wait().expect("poll server"),
                None,
                "server exited while parent process and stdin remained alive"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn server_survives_spawning_thread_exit() {
    let mut server = std::thread::spawn(McpChild::start)
        .join()
        .expect("launcher thread");
    server.assert_alive_for(Duration::from_secs(2));
    let response = server.request(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "ping"
    }));
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"], serde_json::json!({}));
}

#[test]
fn research_failures_are_mcp_tool_errors() {
    let mut server = McpChild::start();
    let cases = [
        ("topic", "VERTEX_PROJECT"),
        ("person", "VERTEX_PROJECT"),
        ("company", "VERTEX_PROJECT"),
        ("market", "VERTEX_PROJECT"),
        ("code", "no sources found"),
        ("jobs", "no [job-profile]"),
    ];
    for (index, (target, expected)) in cases.iter().enumerate() {
        let response = server.request(serde_json::json!({
            "jsonrpc": "2.0", "id": index + 2, "method": "tools/call",
            "params": {"name": "research", "arguments": {
                "query": "Rust official documentation", "target": target,
                "mode": "quick", "max_queries": 1, "max_sources": 1,
                "aspects": ["unsupported-test-aspect"]
            }}
        }));
        assert_eq!(response["result"]["isError"], true, "{target}: {response}");
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("error text");
        assert!(text.contains(expected), "{target}: {text}");
    }
}

#[test]
#[ignore = "subprocess fixture for server_exits_when_parent_process_dies"]
fn lifecycle_parent_fixture() {
    assert_eq!(
        std::env::var("RESEARCHER_LIFECYCLE_FIXTURE").as_deref(),
        Ok("1")
    );
    let mut server = McpChild::start();
    // Keep the MCP write end open after this parent exits, so EOF cannot make
    // the parent-monitor test pass by itself.
    let keeper = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::from(server.stdin.take().expect("stdin")))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start pipe keeper");
    println!("LIFECYCLE_PIDS={} {}", server.child.id(), keeper.id());
    std::io::stdout().flush().expect("flush pids");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .expect("wait for parent-death trigger");
    // Bypass McpChild::drop: the watcher must clean up the orphan.
    std::process::exit(0);
}

#[test]
fn server_exits_when_parent_process_dies() {
    let mut parent = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--ignored",
            "--exact",
            "lifecycle_parent_fixture",
            "--nocapture",
        ])
        .env("RESEARCHER_LIFECYCLE_FIXTURE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn parent fixture");
    let stdout = parent.stdout.take().expect("parent stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("read fixture line");
            if let Some(pids) = line.strip_prefix("LIFECYCLE_PIDS=") {
                let _ = tx.send(pids.to_string());
                break;
            }
        }
    });
    let pids = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("fixture ready");
    let pids: Vec<u32> = pids
        .split_whitespace()
        .map(|p| p.parse().expect("pid"))
        .collect();
    let server_pid = pids[0];
    let keeper_pid = pids[1];
    let is_running = |pid| {
        // An orphan may remain a zombie until the host's reaper runs.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| s.rsplit_once(") ").map(|(_, rest)| !rest.starts_with('Z')))
            .unwrap_or(false)
    };
    assert!(
        is_running(server_pid),
        "server must be alive before parent death"
    );
    writeln!(parent.stdin.as_mut().expect("parent stdin"), "exit").expect("trigger parent exit");
    assert!(parent.wait().expect("wait for parent").success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while is_running(server_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let server_still_running = is_running(server_pid);
    let keeper_still_running = is_running(keeper_pid);
    // SAFETY: kill takes integer PIDs and a valid signal, with no pointers.
    unsafe {
        if server_still_running {
            libc::kill(server_pid as i32, libc::SIGKILL);
        }
        libc::kill(keeper_pid as i32, libc::SIGKILL);
    }
    assert!(keeper_still_running, "stdin holder must outlive the parent");
    assert!(
        !server_still_running,
        "orphaned server did not exit with stdin still open"
    );
}
