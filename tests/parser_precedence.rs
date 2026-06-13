mod parser_support;

use parser_support::assert_snapshot;

#[test]
fn precedence_and_associativity_snapshot() -> Result<(), String> {
    assert_snapshot(
        "return 2^3^2 .. \"x\" .. \"y\" or a and b",
        "Chunk\n  Block span=1:1-1:37\n    Return span=1:1-1:37\n      Binary span=1:8-1:37 op=Or\n        Binary span=1:8-1:26 op=Concat\n          Binary span=1:8-1:12 op=Pow\n            Number span=1:8-1:8 lexeme=2\n            Binary span=1:10-1:12 op=Pow\n              Number span=1:10-1:10 lexeme=3\n              Number span=1:12-1:12 lexeme=2\n          Binary span=1:17-1:26 op=Concat\n            String span=1:17-1:19 bytes=[120]\n            String span=1:24-1:26 bytes=[121]\n        Binary span=1:31-1:37 op=And\n          Name span=1:31-1:31 name=a\n          Name span=1:37-1:37 name=b\n",
    )
}
