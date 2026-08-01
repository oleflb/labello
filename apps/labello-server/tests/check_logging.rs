use std::process::{Command, Stdio};

#[test]
fn check_logging_uses_runtime_parsers_without_starting_the_server() {
    let server = env!("CARGO_BIN_EXE_labello-server");

    let valid = Command::new(server)
        .arg("--check-logging")
        .env("LABELLO_CONFIG", std::env::temp_dir())
        .env("RUST_LOG", "labello_api=debug")
        .env("LABELLO_LOG_FORMAT", "json")
        .status()
        .unwrap();
    assert!(valid.success());

    let invalid_filter = Command::new(server)
        .arg("--check-logging")
        .env("RUST_LOG", "labello_api=[")
        .env("LABELLO_LOG_FORMAT", "text")
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!invalid_filter.success());

    let invalid_format = Command::new(server)
        .arg("--check-logging")
        .env("RUST_LOG", "labello_api=info")
        .env("LABELLO_LOG_FORMAT", "pretty")
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!invalid_format.success());
}
