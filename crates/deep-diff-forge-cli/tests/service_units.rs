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

#[test]
fn the_unit_readers_are_exact() {
    assert_eq!(unit_value("A=1\nB=2\nB=3\n", "B"), "2");
    assert_eq!(unit_value("AB=1\nA=2\n", "A"), "2");
    assert_eq!(
        plist_after("<key>X</key>\n<integer>40</integer>", "X"),
        "40"
    );
}
