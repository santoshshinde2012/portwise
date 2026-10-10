//! End-to-end tests for the `holdmap` binary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::net::TcpListener;
use std::time::Duration;

/// The binary with colour off, no Docker and a throwaway state directory, so tests that stop
/// processes never write to the developer's real history. Tests that check state pass their own
/// `HOLDMAP_HOME` (the later `env` wins).
fn hm() -> Command {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let home = HOME.get_or_init(|| tempfile::tempdir().unwrap());
    let mut c = Command::cargo_bin("holdmap").unwrap();
    c.env("HOLDMAP_COLOR", "never")
        .env("HOLDMAP_HOME", home.path())
        .arg("--no-docker");
    c
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn help_mentions_examples_and_exit_codes() {
    hm().arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("EXAMPLES"))
        .stdout(predicate::str::contains("EXIT CODES"));
}

#[test]
fn list_json_is_valid_and_contains_our_listener() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let out = hm().args(["list", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let entries = v["entries"].as_array().unwrap();
    let ours = entries
        .iter()
        .find(|e| e["port"] == port)
        .expect("our listener is listed");
    assert_eq!(ours["protocol"], "tcp");
    assert_eq!(ours["pid"], std::process::id());
}

#[test]
fn list_plain_table_has_header() {
    let _l = TcpListener::bind("127.0.0.1:0").unwrap();
    hm().args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PORT"));
}

#[test]
fn explain_free_port_succeeds() {
    let port = free_port();
    hm().args(["explain", &port.to_string()])
        .assert()
        .success()
        .stdout(predicate::str::contains("is free"));
}

#[test]
fn explain_busy_port_names_the_owner() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let out = hm()
        .args(["explain", &port.to_string(), "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["status"], "busy");
    assert!(v["headline"]
        .as_str()
        .unwrap()
        .contains(&std::process::id().to_string()));
}

#[test]
fn stop_refuses_to_kill_itself_or_its_parent() {
    // The listener belongs to the test process, which is holdmap's parent: must be blocked.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    hm().args(["stop", &port.to_string(), "--yes"])
        .assert()
        .code(3);
    // …and we're still alive with the socket open.
    assert!(l.local_addr().is_ok());
}

#[test]
fn stop_dry_run_on_free_port_reports_nothing_to_do() {
    let port = free_port();
    hm().args(["stop", &port.to_string(), "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("free").or(predicate::str::contains("Nothing")));
}

#[test]
fn free_port_returns_bindable_ports() {
    let out = hm()
        .args(["free-port", "--count", "3", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let ports = v["ports"].as_array().unwrap();
    assert_eq!(ports.len(), 3);
    for p in ports {
        TcpListener::bind(("127.0.0.1", p.as_u64().unwrap() as u16)).expect("port is free");
    }
}

#[test]
fn free_port_near_skips_busy() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let out = hm()
        .args(["free-port", "--near", &port.to_string()])
        .output()
        .unwrap();
    let got: u16 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    assert!(got > port);
}

#[test]
fn wait_times_out_with_exit_1() {
    let port = free_port();
    hm().args(["wait", &port.to_string(), "--timeout", "300ms", "--quiet"])
        .timeout(Duration::from_secs(10))
        .assert()
        .code(1);
}

#[test]
fn wait_succeeds_when_listening() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    hm().args(["wait", &port.to_string(), "--timeout", "3s"])
        .assert()
        .success();
}

#[test]
fn wait_free_succeeds_on_free_port() {
    let port = free_port();
    hm().args(["wait", &port.to_string(), "--free", "--timeout", "1s", "-q"])
        .assert()
        .success();
}

#[test]
fn bad_target_is_a_usage_error() {
    hm().args(["explain", "not-a-port"]).assert().failure();
}

#[test]
fn stop_and_kill_reject_out_of_range_ports() {
    for (cmd, port) in [("stop", "0"), ("stop", "99999"), ("kill", ":0")] {
        hm().args([cmd, port, "--dry-run"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("not a valid port (1–65535)"));
    }
}

#[test]
fn tui_without_a_terminal_is_a_clean_error_not_a_panic() {
    hm().arg("tui")
        .write_stdin("")
        .timeout(Duration::from_secs(10))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs an interactive terminal"))
        .stderr(predicate::str::contains("panicked").not());
}

#[test]
fn completions_and_man_render() {
    hm().args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("holdmap"));
    hm().arg("man")
        .assert()
        .success()
        .stdout(predicate::str::contains(".TH"));
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::process::{Child, Stdio};

    fn python_listener(port: u16, ignore_term: bool) -> Option<Child> {
        let code = format!(
            "import socket,signal,time\n{}s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)\ns.bind(('127.0.0.1',{port}));s.listen()\ntime.sleep(60)",
            if ignore_term { "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n" } else { "" }
        );
        let child = std::process::Command::new("python3")
            .args(["-c", &code])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Some(child);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    #[test]
    fn stop_frees_a_real_listener() {
        let port = free_port();
        let Some(mut child) = python_listener(port, false) else {
            eprintln!("python3 not available; skipping");
            return;
        };
        hm().args(["stop", &port.to_string(), "--yes"])
            .timeout(Duration::from_secs(20))
            .assert()
            .success()
            .stdout(predicate::str::contains("free"));
        let _ = child.wait();
        TcpListener::bind(("127.0.0.1", port)).expect("port was freed");
    }

    #[test]
    fn stop_escalates_to_sigkill_when_sigterm_is_ignored() {
        let port = free_port();
        let Some(mut child) = python_listener(port, true) else {
            return;
        };
        hm().args([
            "stop",
            &port.to_string(),
            "--yes",
            "--timeout",
            "500ms",
            "--json",
        ])
        .timeout(Duration::from_secs(20))
        .assert()
        .success()
        .stdout(predicate::str::contains("SIGKILL"));
        let _ = child.wait();
    }

    /// Stops whatever is left on a port when a stack test ends, even if it failed half-way.
    struct Cleanup(u16, std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = hm()
                .env("HOLDMAP_HOME", &self.1)
                .args(["stop", &self.0.to_string(), "--yes", "--force"])
                .timeout(Duration::from_secs(10))
                .output();
        }
    }

    #[test]
    fn project_stack_up_status_down() {
        let port = free_port();
        let dir = home();
        let state = home();
        std::fs::write(
            dir.path().join(".holdmap.toml"),
            format!(
                "name = \"e2e\"\n\n[services.web]\nport = {port}\ncommand = \"exec python3 -m http.server $PORT --bind 127.0.0.1\"\nhealth = \"/\"\nready_timeout_s = 20\n"
            ),
        )
        .unwrap();
        let _guard = Cleanup(port, state.path().to_path_buf());
        let file = dir.path().to_str().unwrap();
        let run = |args: &[&str]| {
            let out = hm()
                .env("HOLDMAP_HOME", state.path())
                .current_dir(dir.path())
                .args(args)
                .timeout(Duration::from_secs(30))
                .output()
                .unwrap();
            (
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).to_string(),
                String::from_utf8_lossy(&out.stderr).to_string(),
            )
        };
        if std::process::Command::new("python3")
            .arg("-V")
            .output()
            .is_err()
        {
            return;
        }

        let (code, out, err) = run(&["up", "--file", file]);
        assert_eq!(code, Some(0), "up failed: {out}{err}");
        assert!(out.contains("web") && out.contains("up after"), "{out}");

        // Already running: nothing to do, still success.
        let (code, out, _) = run(&["up"]);
        assert_eq!(code, Some(0));
        assert!(out.contains("already running"), "{out}");

        let (code, out, err) = run(&["status", "--json"]);
        assert_eq!(code, Some(0), "{out}{err}");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let web = &v["services"][0];
        assert_eq!(web["name"], "web");
        assert_eq!(web["port"], port);
        assert_eq!(web["state"], "running");
        assert_eq!(web["http"]["status"], 200);

        let (code, out, _) = run(&["down", "--dry-run"]);
        assert_eq!(code, Some(0));
        assert!(out.contains(&format!(":{port}")), "{out}");
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_ok(),
            "dry run stopped it"
        );

        let (code, out, err) = run(&["down", "--yes"]);
        assert_eq!(code, Some(0), "down failed: {out}{err}");
        let mut released_port = None;
        for _ in 0..50 {
            if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
                released_port = Some(listener);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Keep the successful bind: dropping it and binding again lets a parallel test
        // select this newly free ephemeral port between the two binds.
        assert!(
            released_port.is_some(),
            "down did not free the port within 5s"
        );
        drop(released_port);

        let (code, _, _) = run(&["status", "--no-http"]);
        assert_eq!(code, Some(1), "status is non-zero when a service is down");
    }

    #[test]
    fn up_skips_only_the_dependents_of_a_service_that_fails() {
        let (db, api, docs) = (free_port(), free_port(), free_port());
        let dir = home();
        std::fs::write(
            dir.path().join(".holdmap.toml"),
            format!(
                "[services.db]\nport = {db}\n\n[services.api]\nport = {api}\ncommand = \"true\"\ndepends_on = [\"db\"]\n\n[services.docs]\nport = {docs}\ncommand = \"true\"\n"
            ),
        )
        .unwrap();
        hm().env("HOLDMAP_HOME", dir.path())
            .current_dir(dir.path())
            .args(["up", "--dry-run"])
            .timeout(Duration::from_secs(20))
            .assert()
            .code(1)
            .stderr(predicate::str::contains("db"))
            .stderr(predicate::str::contains("skipped: it depends on db"))
            .stdout(predicate::str::contains("docs"))
            .stdout(predicate::str::contains("would run"));
    }

    #[test]
    fn up_refuses_a_port_held_by_something_else() {
        let port = free_port();
        let Some(mut child) = python_listener(port, false) else {
            return;
        };
        let dir = home();
        std::fs::write(
            dir.path().join(".holdmap.toml"),
            format!("[services.api]\nport = {port}\ncommand = \"true\"\n"),
        )
        .unwrap();
        hm().env("HOLDMAP_HOME", dir.path())
            .current_dir(dir.path())
            .args(["up"])
            .timeout(Duration::from_secs(20))
            .assert()
            .code(1)
            .stderr(predicate::str::contains("--replace"));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn stop_all_dev_dry_run_lists_a_dev_server() {
        let port = free_port();
        let mut child = std::process::Command::new("python3")
            .args([
                "-m",
                "http.server",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let out = hm()
            .args(["stop", "--all-dev", "--dry-run", "--json"])
            .timeout(Duration::from_secs(20))
            .output()
            .unwrap();
        let _ = child.kill();
        let _ = child.wait();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(&format!(":{port}")), "{text}");
    }

    #[test]
    fn run_frees_port_then_execs_with_port_env() {
        let port = free_port();
        let Some(mut child) = python_listener(port, false) else {
            return;
        };
        hm().args([
            "run",
            "-p",
            &port.to_string(),
            "--yes",
            "--",
            "sh",
            "-c",
            "echo got=$PORT",
        ])
        .timeout(Duration::from_secs(20))
        .assert()
        .success()
        .stdout(predicate::str::contains(format!("got={port}")));
        let _ = child.wait();
    }
}

fn home() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn graph_exports_json_dot_and_mermaid() {
    let out = hm().args(["graph", "--all", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["nodes"].is_array() && v["edges"].is_array() && v["clusters"].is_array());
    hm().args(["graph", "--all", "--dot"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("digraph holdmap {"));
    hm().args(["mesh", "--all", "--format", "mermaid"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("flowchart LR"));
    hm().args(["graph", "--cluster", "definitely-not-a-cluster"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no cluster named"));
}

#[test]
fn graph_shows_a_live_connection_between_two_processes() {
    if std::process::Command::new("python3")
        .arg("-V")
        .output()
        .is_err()
    {
        return;
    }
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let mut client = std::process::Command::new("python3")
        .args([
            "-c",
            &format!("import socket,time; s=socket.create_connection(('127.0.0.1',{port})); print('ok', flush=True); time.sleep(30)"),
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let (_conn, _) = l.accept().unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(client.stdout.as_mut().unwrap()),
        &mut line,
    )
    .unwrap();
    let out = hm().args(["graph", "--all", "--json"]).output().unwrap();
    let _ = client.kill();
    let _ = client.wait();
    let g: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let client_id = format!("svc:{}", client.id());
    let edge = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["from"] == client_id.as_str() && e["port"] == port)
        .unwrap_or_else(|| panic!("no edge from {client_id} to :{port}: {}", g["edges"]));
    assert_eq!(edge["kind"], "local");
}

#[test]
fn pins_round_trip_in_an_isolated_home() {
    let h = home();
    hm().env("HOLDMAP_HOME", h.path())
        .args(["pin", "3999", "--label", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Pinned :3999"));
    let out = hm()
        .env("HOLDMAP_HOME", h.path())
        .args(["pins", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v[0]["port"], 3999);
    assert_eq!(v[0]["label"], "demo");
    hm().env("HOLDMAP_HOME", h.path())
        .args(["pin", "3999"])
        .assert()
        .success()
        .stdout(predicate::str::contains(":3999 is already pinned"));
    hm().env("HOLDMAP_HOME", h.path())
        .args(["unpin", "3999"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unpinned :3999"));
    hm().env("HOLDMAP_HOME", h.path())
        .args(["unpin", "3999"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains(":3999 isn't pinned"));
    hm().env("HOLDMAP_HOME", h.path())
        .args(["pins", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

#[test]
fn history_restart_and_open() {
    let h = home();
    hm().env("HOLDMAP_HOME", h.path())
        .args(["history", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
    let port = free_port();
    hm().env("HOLDMAP_HOME", h.path())
        .args(["restart", &port.to_string()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no record"));
    hm().args(["open", "--print", "3000"])
        .assert()
        .success()
        .stdout("http://localhost:3000\n");
}

#[test]
fn stop_unknown_cluster_is_nothing_to_stop() {
    hm().args(["stop", "--cluster", "nope", "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("No cluster named"));
}

#[cfg(target_os = "linux")]
#[test]
fn ssh_agentless_local_lists_ports() {
    if std::process::Command::new("ss").arg("-V").output().is_err() {
        return;
    }
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let out = hm()
        .args(["ssh", "local", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["platform"], "remote:local");
    assert!(v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["port"] == port));
}

#[test]
fn watch_runs_for_a_bounded_number_of_polls() {
    hm().args(["watch", "--json", "--polls", "2", "--interval", "50ms"])
        .assert()
        .success();
}

#[test]
fn hint_explains_a_busy_port_and_stays_quiet_otherwise() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    hm().args([
        "hint",
        "--exit-code",
        "1",
        "--",
        &format!("npm run dev -- --port {port}"),
    ])
    .assert()
    .success()
    .stderr(predicate::str::contains(format!("holdmap stop {port}")));
    // A client command, or a server whose port is free, prints nothing.
    hm().args([
        "hint",
        "--exit-code",
        "1",
        "--",
        &format!("curl localhost:{port}"),
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty());
    let free = free_port();
    hm().args([
        "hint",
        "--exit-code",
        "1",
        "--",
        &format!("vite --port {free}"),
    ])
    .assert()
    .success()
    .stderr(predicate::str::is_empty());
}

#[test]
fn init_prints_shell_hooks() {
    for (shell, needle) in [
        ("zsh", "add-zsh-hook"),
        ("bash", "PROMPT_COMMAND"),
        ("fish", "fish_postexec"),
        ("pwsh", "LASTEXITCODE"),
    ] {
        hm().args(["init", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains(needle))
            .stdout(predicate::str::contains("holdmap hint"));
    }
}

#[test]
fn project_file_errors_are_reported_with_the_path() {
    let dir = home();
    std::fs::write(
        dir.path().join(".holdmap.toml"),
        "[services.web]\nport = 3000\nhealth = \"nope\"\n",
    )
    .unwrap();
    hm().current_dir(dir.path())
        .env("HOLDMAP_HOME", dir.path())
        .args(["status"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(".holdmap.toml"));
}

#[test]
fn init_project_writes_a_valid_file() {
    let dir = home();
    hm().current_dir(dir.path())
        .env("HOLDMAP_HOME", dir.path())
        .args(["init"])
        .assert()
        .success();
    let text = std::fs::read_to_string(dir.path().join(".holdmap.toml")).unwrap();
    assert!(text.contains("name ="), "{text}");
    // A second run refuses to overwrite.
    hm().current_dir(dir.path())
        .env("HOLDMAP_HOME", dir.path())
        .args(["init"])
        .assert()
        .failure();
}
