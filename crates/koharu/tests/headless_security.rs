use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

#[test]
fn missing_secret_exits_nonzero_without_listening() {
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let data_root = std::env::temp_dir().join(format!(
        "koharu-headless-security-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_koharu"))
        .args(["--headless", "--port", &port.to_string()])
        .env_remove("KOHARU_AUTH_SECRET")
        .env("KOHARU_DATA_ROOT", &data_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let started_at = Instant::now();
    let deadline = started_at + Duration::from_secs(30);
    let mut connected = false;
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        connected |= TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().unwrap(),
            Duration::from_millis(10),
        )
        .is_ok();
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        let pid = child.id();
        child.kill().unwrap();
        let output = child.wait_with_output().unwrap();
        panic!(
            "headless process {pid} did not reject the missing secret within {:?}; connected={connected}; stdout={:?}; stderr={:?}",
            started_at.elapsed(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "missing secret must fail");
    assert!(
        !connected,
        "server listened before rejecting missing secret"
    );
    assert!(stderr.contains("headless mode requires KOHARU_AUTH_SECRET"));
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());

    let _ = std::fs::remove_dir_all(data_root);
}
