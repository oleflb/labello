#[cfg(unix)]
#[test]
fn sigterm_uses_the_graceful_shutdown_path() {
    use std::{
        fs,
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(std::path::PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("labello-sigterm-{}-{unique}", std::process::id())),
    );
    fs::create_dir(&directory.0).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_labello-server"))
        .current_dir(&directory.0)
        .env("LABELLO_BIND", "127.0.0.1:0")
        .env_remove("RUST_LOG")
        .env_remove("LABELLO_LOG_FORMAT")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lines = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("server did not start; logs: {lines:?}");
        }
        let line = receiver.recv_timeout(remaining).unwrap();
        let started = line.contains("server.started");
        lines.push(line);
        if started {
            break;
        }
    }
    // `server.started` is emitted immediately before Axum first polls the
    // graceful-shutdown future and installs the OS signal handlers.
    std::thread::sleep(Duration::from_millis(100));

    let signal_status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(signal_status.success());
    let status = child.wait().unwrap();
    assert!(status.success(), "server exit status: {status}");
    lines.extend(receiver.iter());
    reader.join().unwrap();

    assert!(
        lines
            .iter()
            .any(|line| line.contains("server.shutdown.started")),
        "logs: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("server.stopped")),
        "logs: {lines:?}"
    );
}
