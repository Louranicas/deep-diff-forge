//! The SDK's public signatures, pinned. A change here is a breaking change to the facade and must
//! be made deliberately, with a version bump; the compiler checks every line.
use deep_diff_forge_sdk::{
    Error, Guards, InputSeal, RankedFile, Review, ReviewFile, exit, rank, review,
};
use serde_json::Value;
use std::path::Path;

#[test]
fn public_signatures_are_pinned() {
    let _: fn(&str, Guards) -> Result<Review, Error> = review;
    let _: fn(&Review) -> Vec<RankedFile> = rank;
    let _: fn(&Review) -> String = Review::to_json;
    let _: fn(&Review) -> &[ReviewFile] = Review::files;
    let _: fn(&Review) -> &InputSeal = Review::seal;
    let _: fn() -> Guards = Guards::none;
    let _: fn() -> Guards = Guards::all;
    let _: fn(Guards) -> Guards = Guards::require_files;
    let _: fn(Guards) -> Guards = Guards::require_hunks;
    let _: fn(&Error) -> i32 = Error::exit_code;
    let _: fn(&str, i32) -> Option<&'static exit::ExitCode> = exit::lookup;
    let _: &[exit::ExitCode] = exit::EXIT_CODES;
    #[cfg(unix)]
    {
        use deep_diff_forge_sdk::Client;
        let _: fn(&Path) -> Client = Client::at;
        let _: fn(&Client, &str, &Value) -> Result<Value, Error> = Client::call;
    }
    // Error is #[non_exhaustive]: callers must keep a wildcard arm.
    let describe = |e: &Error| match e {
        Error::Parse(_) => "parse",
        Error::Refused(_) => "refused",
        Error::Rpc { .. } => "rpc",
        Error::Transport(_) => "transport",
        _ => "other",
    };
    assert_eq!(describe(&Error::Refused("x")), "refused");
    let _ = (Path::new("."), Value::Null);
}
