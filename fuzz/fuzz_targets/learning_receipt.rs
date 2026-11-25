#![no_main]
// DDF 0.11.1: the receipt-boundary JSON that `learn record --stdin` reads through the learning
// crate's production parser. Design/status: docs/TESTING_GOLD_STANDARD.md (fuzz targets).
//
// The property, for any UTF-8 byte string under the target cap: no panic in receipt parsing; a
// receipt that parses serializes and parses again as the same typed value.
use deep_diff_forge_learning::StrategyReceipt;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 1024 * 1024 {
        return;
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(receipt) = StrategyReceipt::from_json(text.trim()) else {
        return;
    };
    let encoded = receipt
        .to_json()
        .expect("a parsed strategy receipt serializes");
    let again = StrategyReceipt::from_json(&encoded).expect("the serialized receipt parses");
    assert_eq!(again, receipt);
});
