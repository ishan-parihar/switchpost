// ─── CLI boot smoke test: the COMPILED binary must boot and integrate ──
//
// Post-compilation gate. `cargo check` only proves the tree compiles; this
// test proves the artifact that ships actually works end to end:
//
//   1. the binary boots on a scratch DB, runs migrations, seeds the user,
//      and answers /health + /ready (HTTP server + scheduler init);
//   2. the embedded SPA is served from inside the binary (rust-embed) —
//      a stale binary serves a stale frontend no matter how correct the
//      source is, so grep the SERVED bytes, not the binary;
//   3. the password gate rejects a wrong password and sets sf_session on
//      the right one;
//   4. an authenticated REST write (create tag) then read works;
//   5. after the server stops, the CLI — the SAME binary, pointed at the
//      SAME database — lists the tag the API created. That is the
//      CLI ↔ REST ↔ SQLite integration in one compiled artifact.
//
// The DB lives under target/test-data (never the operator's data dir), and
// the server is killed before the CLI runs so the single-writer lock is
// free — the same reason the live VPS refuses a second process on its DB.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A free loopback port: bind :0, read the port, drop the listener.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind :0")
        .local_addr()
        .expect("local addr")
        .port()
}

/// One raw HTTP/1.1 request over a fresh connection. `Connection: close`
/// makes the server hang up after the response, so read-to-EOF is the body.
/// Returns (status, headers, body); transport errors come back as Err so a
/// polling loop can tolerate the server not being up YET (connecting before
/// bind is refused, and a panic here would abort the whole test binary).
fn http(port: u16, request: &str) -> std::io::Result<(u16, String, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(request.as_bytes())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok((status, head.to_string(), body.to_string()))
}

fn get(port: u16, path: &str, cookie: Option<&str>) -> std::io::Result<(u16, String, String)> {
    let cookie = cookie
        .map(|c| format!("Cookie: {c}\r\n"))
        .unwrap_or_default();
    http(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{cookie}Connection: close\r\n\r\n"),
    )
}

fn post_json(
    port: u16,
    path: &str,
    body: &str,
    cookie: Option<&str>,
    origin: &str,
) -> std::io::Result<(u16, String, String)> {
    let cookie = cookie
        .map(|c| format!("Cookie: {c}\r\n"))
        .unwrap_or_default();
    http(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cookie}Connection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn boot_server(scratch: &std::path::Path, port: u16) -> Server {
    let db = scratch.join("smoke.db");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    // stderr goes to a file: if the child dies mid-test the log is the only
    // evidence of why, and a piped stdout nobody drains would deadlock it.
    let log = std::fs::File::create(scratch.join("serve.log")).expect("serve.log");
    let child = Command::new(env!("CARGO_BIN_EXE_switchpost"))
        .args(["serve", "--port", &port.to_string()])
        .env("DATABASE_URL", &url)
        .env("APP_PASSWORD", "cli-smoke-pass")
        .env("SWITCHPOST_DIR", scratch)
        // APP_URL's scheme picks HTTP vs TLS (main.rs: starts_with("https://")),
        // and its origin is what the CSRF middleware accepts — keep both on the
        // plain-HTTP loopback origin so the raw-socket client in this test works.
        .env("APP_URL", format!("http://127.0.0.1:{port}"))
        .env("FRONTEND_URL", format!("http://127.0.0.1:{port}"))
        .env("BIND_HOST", "127.0.0.1")
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .expect("the compiled binary must spawn `serve`");
    Server { child, port }
}

/// Poll /health until it answers 200 or the deadline passes.
fn wait_healthy(scratch: &std::path::Path, port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok((status, _, body)) = get(port, "/health", None) {
            if status == 200 && body.contains("\"ok\"") {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let log = std::fs::read_to_string(scratch.join("serve.log")).unwrap_or_default();
    eprintln!("--- serve.log tail ---\n{}", &log[..log.len().min(2000)]);
    false
}

#[test]
fn compiled_binary_boots_serves_logs_in_and_integrates_with_the_cli() {
    // Unique per RUN, not per pid: a panicked earlier run can leave a serve
    // child (and its writer-lock file) behind, and pid reuse would collide.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let scratch = std::path::Path::new("target/test-data")
        .join(format!("cli-boot-smoke-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("scratch dir");

    let port = free_port();
    let server = boot_server(&scratch, port);

    // 1. boots, migrates, answers health
    assert!(
        wait_healthy(&scratch, port),
        "the compiled binary must answer /health within 30s on port {port}"
    );
    let (status, _, body) = get(port, "/ready", None).expect("GET /ready");
    assert_eq!(status, 200, "/ready must answer 200 once booted: {body}");

    // 2. the embedded SPA is served from the binary itself
    let (status, _, body) = get(port, "/", None).expect("GET /");
    assert_eq!(status, 200, "GET / must serve the SPA: {body}");
    assert!(
        body.contains("Switchpost"),
        "the served index must be the embedded frontend (title 'Switchpost'), got: {:.200}",
        body
    );

    // 3. the password gate: wrong password 401, right password sets a session
    let origin = format!("http://127.0.0.1:{port}");
    let (status, _, _) = post_json(port, "/api/auth/login", r#"{"password":"wrong"}"#, None, &origin)
        .expect("POST login");
    assert_eq!(status, 401, "a wrong password must be rejected");
    let (status, headers, _) = post_json(
        port,
        "/api/auth/login",
        r#"{"password":"cli-smoke-pass"}"#,
        None,
        &origin,
    )
    .expect("POST login");
    assert_eq!(status, 200, "the right password must log in");
    let cookie = headers
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
        .and_then(|v| v.split(';').next().map(|s| s.trim().to_string()))
        .filter(|c| c.starts_with("sf_session="))
        .unwrap_or_else(|| panic!("login must set sf_session, headers: {headers}"));

    // 4. authenticated REST write + read (a tag needs no integration, so it
    //    works on a fresh DB; a post would need a connected channel)
    let marker = format!("cli-boot-smoke-{nanos}");
    let body = format!(r#"{{"name":"{marker}"}}"#);
    let (status, _, resp) =
        post_json(port, "/api/tags", &body, Some(&cookie), &origin).expect("POST /api/tags");
    assert_eq!(status, 200, "creating a tag must succeed: {resp}");
    let created: serde_json::Value =
        serde_json::from_str(&resp).unwrap_or_else(|e| panic!("create response must be JSON ({e}): {resp}"));
    let tag_id = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("created tag must carry an id: {resp}"))
        .to_string();

    let (status, _, resp) = get(port, "/api/tags", Some(&cookie)).expect("GET /api/tags");
    assert_eq!(status, 200, "listing tags must be authenticated-200");
    assert!(resp.contains(&tag_id), "the created tag must be listed: {resp}");
    assert!(resp.contains(&marker), "the created tag name must be listed: {resp}");

    // 5. server stops, then the CLI (same binary, same DB) sees the row
    drop(server);
    std::thread::sleep(Duration::from_millis(500));

    let db = scratch.join("smoke.db");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let out = Command::new(env!("CARGO_BIN_EXE_switchpost"))
        .args(["--json", "tags", "list"])
        .env("DATABASE_URL", &url)
        .env("APP_PASSWORD", "cli-smoke-pass")
        .output()
        .expect("the compiled binary must run `tags list`");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "CLI tags list must exit 0: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("--json tags list must emit JSON ({e}): {stdout}"));
    let listed = parsed["data"].as_array().cloned().unwrap_or_default();
    assert!(
        listed.iter().any(|t| t["id"].as_str() == Some(tag_id.as_str())),
        "the CLI must list the tag the REST API created (id {tag_id}): {stdout}"
    );
    assert!(
        listed.iter().any(|t| t["name"].as_str() == Some(marker.as_str())),
        "the CLI row must carry the tag name: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
