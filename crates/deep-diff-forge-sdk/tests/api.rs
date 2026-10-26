//! The SDK's public signatures, pinned. A change here is a breaking change to the facade and must
//! be made deliberately, with a version bump; the compiler checks every line.
use deep_diff_forge_sdk::{Client, Error, InputGuards, RankedFile, Review, exit, rank, review};
use serde_json::Value;
use std::path::Path;

#[test]
fn public_signatures_are_pinned() {
    let _: fn(&str, InputGuards) -> Result<Review, Error> = review;
    let _: fn(&Review) -> Vec<RankedFile> = rank;
    let _: fn(&Review) -> String = Review::to_json;
    let _: fn(&Path) -> Client = Client::at;
    let _: fn(&Client, &str, &Value) -> Result<Value, Error> = Client::call;
    let _: fn(&Error) -> i32 = Error::exit_code;
    let _: fn(&str, i32) -> Option<&'static exit::ExitCode> = exit::lookup;
    let _: &[exit::ExitCode] = exit::EXIT_CODES;
    let _ = InputGuards {
        files: true,
        hunks: true,
    };
    let _ = |e: Error| match e {
        Error::Parse(_) | Error::Refused(_) | Error::Rpc { .. } | Error::Transport(_) => (),
    };
    let r: Review = review("", InputGuards::default()).unwrap();
    let _: (
        &Vec<deep_diff_forge_sdk::ReviewFile>,
        &deep_diff_forge_sdk::InputSeal,
    ) = (&r.files, &r.seal);
}
