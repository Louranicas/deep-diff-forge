//! `Client` against a real daemon, run in-process on a private socket: the daemon's
//! `diff.compute` and the facade's `review` give the same document, and errors keep their codes.
use deep_diff_forge_sdk::{Client, Error, InputGuards, review};
use serde_json::{Value, json};
use std::os::unix::fs::DirBuilderExt as _;

#[test]
fn the_client_and_the_facade_agree_with_a_live_daemon() {
    let dir = std::env::temp_dir().join(format!("ddf-sdk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let socket = dir.join("d.sock");
    let location = deep_diff_forge_daemon::SocketLocation::at(&socket);
    let server = std::thread::spawn(move || deep_diff_forge_daemon::run_server(&location));
    let client = Client::at(&socket);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while client.call("daemon.health", &Value::Null).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never answered"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let patch = "--- a/x\n+++ b/x\n@@ -1,1 +1,1 @@\n-a\n+b\n";
    let remote = client
        .call("diff.compute", &json!({"patch": patch}))
        .unwrap();
    let local: Value =
        serde_json::from_str(&review(patch, InputGuards::default()).unwrap().to_json()).unwrap();
    assert_eq!(remote, local);

    let refused = client
        .call("diff.compute", &json!({"patch": "", "require_files": true}))
        .unwrap_err();
    assert_eq!(
        refused,
        Error::Rpc {
            code: 7,
            message: "refused: 0 files in input (--require-files)".into()
        }
    );
    let unknown = client.call("no.such", &Value::Null).unwrap_err();
    assert!(
        matches!(unknown, Error::Rpc { code: -32601, .. }),
        "{unknown}"
    );

    assert_eq!(
        client.call("daemon.shutdown", &Value::Null).unwrap()["stopping"],
        true
    );
    server.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
