use std::{process::Stdio, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    process::{Child, ChildStdout, Command},
    time::timeout,
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_ipkvm-browser-fixture");

struct FixtureProcess {
    child: Child,
    stdout: BufReader<ChildStdout>,
}

impl FixtureProcess {
    async fn start() -> Self {
        let mut command = Command::new(FIXTURE);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdout }
    }

    async fn read_line(&mut self) -> String {
        let mut line = String::new();
        let byte_count = timeout(Duration::from_secs(2), self.stdout.read_line(&mut line))
            .await
            .expect("fixture did not produce a line")
            .expect("failed to read fixture stdout");
        assert_ne!(byte_count, 0, "fixture stdout closed early");
        line.trim_end_matches(['\r', '\n']).to_string()
    }

    async fn ready(&mut self) -> String {
        let line = self.read_line().await;
        let mut fields = line.split('\t');
        assert_eq!(fields.next(), Some("READY"));
        let url = fields.next().unwrap().to_string();
        assert_eq!(fields.next(), Some("320"));
        assert_eq!(fields.next(), Some("180"));
        assert_eq!(fields.next(), None);
        url
    }

    async fn stop_with_command(&mut self) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(b"STOP\n").await.unwrap();
        stdin.flush().await.unwrap();
        assert_eq!(self.read_line().await, "STOPPED");
        self.expect_success().await;
    }

    async fn stop_with_eof(&mut self) {
        drop(self.child.stdin.take());
        assert_eq!(self.read_line().await, "STOPPED");
        self.expect_success().await;
    }

    async fn expect_success(&mut self) {
        let status = timeout(Duration::from_secs(2), self.child.wait())
            .await
            .expect("fixture did not exit")
            .expect("failed to wait for fixture");
        assert!(status.success(), "fixture exited with {status}");
    }
}

#[tokio::test]
async fn fixture_serves_novnc_on_a_dynamic_port_and_stops_by_command() {
    let mut fixture = FixtureProcess::start().await;
    let url = fixture.ready().await;
    assert!(url.starts_with("http://127.0.0.1:"));

    let response = http_get(&url, "/vendor/novnc/core/rfb.js").await;
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("content-type: text/javascript; charset=utf-8\r\n"));
    assert!(response.contains("export default class RFB"));

    fixture.stop_with_command().await;
}

#[tokio::test]
async fn fixture_treats_stdin_eof_as_a_normal_shutdown() {
    let mut fixture = FixtureProcess::start().await;
    fixture.ready().await;

    fixture.stop_with_eof().await;
}

#[test]
fn fixture_binary_is_guarded_by_its_required_feature() {
    let metadata = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .unwrap();
    assert!(metadata.status.success());
    let json = String::from_utf8(metadata.stdout).unwrap();
    assert!(json.contains("\"name\":\"ipkvm-browser-fixture\""));
    assert!(json.contains("\"required-features\":[\"browser-fixture\"]"));
}

async fn http_get(base_url: &str, path: &str) -> String {
    let address = base_url.strip_prefix("http://").unwrap();
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8(response).unwrap()
}
