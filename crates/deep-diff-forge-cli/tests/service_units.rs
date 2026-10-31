//! The service units in `contrib/` start the daemon the way this CLI does and give a stop at
//! least the time `daemon stop` waits for the drain, so a service manager does not kill a daemon
//! that is still answering.

use std::path::Path;

fn read(relative: &str) -> String {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    std::fs::read_to_string(root.join(relative)).unwrap()
}

/// The value of the first `key=value` line of a systemd unit.
fn unit_value<'a>(unit: &'a str, key: &str) -> &'a str {
    unit.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no {key}= line"))
}

/// The text of the `<string>` or `<integer>` element right after `<key>name</key>`.
fn plist_after<'a>(plist: &'a str, name: &str) -> &'a str {
    let key = format!("<key>{name}</key>");
    let rest = &plist[plist.find(&key).unwrap_or_else(|| panic!("no {name}")) + key.len()..];
    let start = rest.find('>').unwrap() + 1;
    &rest[start..start + rest[start..].find('<').unwrap()]
}

#[test]
fn the_systemd_unit_starts_and_stops_like_the_cli() {
    let unit = read("contrib/systemd/deep-diff-forge.service");
    let start = unit_value(&unit, "ExecStart");
    let stop = unit_value(&unit, "ExecStop");
    assert!(
        start.ends_with("/deep-diff-forge daemon start --log-format json"),
        "{start}"
    );
    assert!(stop.ends_with("/deep-diff-forge daemon stop"), "{stop}");
    // systemd runs ExecStop after a daemon that exited on its own too; its exit 6 then must not
    // fail the unit (and restart it).
    assert!(
        stop.starts_with('-'),
        "ExecStop must ignore 'no daemon': {stop}"
    );
    let timeout: u64 = unit_value(&unit, "TimeoutStopSec").parse().unwrap();
    assert!(
        timeout > deep_diff_forge_daemon::STOP_WAIT.as_secs(),
        "TimeoutStopSec={timeout} must exceed the stop wait"
    );
    assert_eq!(unit_value(&unit, "Restart"), "on-failure");
    assert_eq!(unit_value(&unit, "Type"), "simple");
}

#[test]
fn the_launchd_agent_starts_like_the_cli() {
    let plist = read("contrib/launchd/dev.deep-diff-forge.daemon.plist");
    let args = &plist[plist.find("<key>ProgramArguments</key>").unwrap()..];
    let args = &args[..args.find("</array>").unwrap()];
    let strings: Vec<&str> = args
        .split("<string>")
        .skip(1)
        .map(|s| &s[..s.find("</string>").unwrap()])
        .collect();
    assert!(strings[0].ends_with("/deep-diff-forge"), "{strings:?}");
    assert_eq!(&strings[1..], ["daemon", "start", "--log-format", "json"]);
    let timeout: u64 = plist_after(&plist, "ExitTimeOut").parse().unwrap();
    assert!(timeout > deep_diff_forge_daemon::STOP_WAIT.as_secs());
    assert!(
        plist.contains("<key>SuccessfulExit</key>\n    <false/>"),
        "only a failure is restarted"
    );
    // macOS has no XDG_RUNTIME_DIR, without which `daemon start` refuses (exit 2) and launchd
    // would restart it every ThrottleInterval.
    let environment = &plist[plist
        .find("<key>EnvironmentVariables</key>")
        .expect("environment")..];
    assert!(plist_after(environment, "XDG_RUNTIME_DIR").starts_with('/'));
}

/// The metrics timer runs the metrics unit, which writes a `.prom` file (the only name the
/// textfile collector reads) with options this CLI admits: run with them and no daemon, it exits 6
/// (no daemon), not 2 (usage).
#[test]
fn the_metrics_units_write_a_textfile_the_cli_way() {
    let unit = read("contrib/systemd/deep-diff-forge-metrics.service");
    assert_eq!(unit_value(&unit, "Type"), "oneshot");
    assert_eq!(unit_value(&unit, "After"), "deep-diff-forge.service");
    let start = unit_value(&unit, "ExecStart");
    let (binary, args) = start.split_once(' ').unwrap();
    assert!(binary.ends_with("/deep-diff-forge"), "{start}");
    let args: Vec<&str> = args.split(' ').collect();
    assert_eq!(
        args[..5],
        ["daemon", "metrics", "--format", "prometheus", "--out"]
    );
    assert_eq!(args.len(), 6, "{start}");
    assert!(
        args[5].starts_with('/')
            && std::path::Path::new(args[5]).extension() == Some("prom".as_ref()),
        "{start}"
    );
    let timer = read("contrib/systemd/deep-diff-forge-metrics.timer");
    assert_eq!(
        unit_value(&timer, "Unit"),
        "deep-diff-forge-metrics.service"
    );
    assert_eq!(unit_value(&timer, "OnUnitActiveSec"), "15s");

    let dir = std::env::temp_dir().join(format!("ddf-units-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("deep-diff-forge.prom");
    let mut admitted = args[..4].to_vec();
    let socket = dir.join("d.sock");
    let (out, socket) = (out.to_str().unwrap(), socket.to_str().unwrap());
    admitted.extend(["--out", out, "--socket", socket]);
    #[expect(
        clippy::disallowed_methods,
        reason = "integration test: runs the CLI with the unit's arguments"
    )]
    let ran = std::process::Command::new(env!("CARGO_BIN_EXE_deep-diff-forge"))
        .args(&admitted)
        .output()
        .unwrap();
    assert_eq!(
        ran.status.code(),
        Some(6),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_unit_readers_are_exact() {
    assert_eq!(unit_value("A=1\nB=2\nB=3\n", "B"), "2");
    assert_eq!(unit_value("AB=1\nA=2\n", "A"), "2");
    assert_eq!(
        plist_after("<key>X</key>\n<integer>40</integer>", "X"),
        "40"
    );
}
