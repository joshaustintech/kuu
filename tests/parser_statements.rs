mod parser_support;

use parser_support::assert_snapshot;

#[test]
fn globals_labels_and_methods_snapshot() -> Result<(), String> {
    assert_snapshot(
        "global<const> *\nlocal x<close> = 1\n::loop::\ngoto loop\nfunction t.a:b(c)\n  return c\nend\n",
        "Chunk\n  Block span=1:1-7:3\n    GlobalAll span=1:1-1:15\n      Attribute span=1:7-1:13 name=const\n    LocalDecl span=2:1-2:18\n      AttributedName span=2:7-2:14 name=x\n        Attribute span=2:8-2:14 name=close\n      Number span=2:18-2:18 lexeme=1\n    Label span=3:1-3:8 name=loop\n    Goto span=4:1-4:9 name=loop\n    Function span=5:1-7:3\n      FunctionName span=5:10-5:14 prefix=[\"t\", \"a\"] method=Some(\"b\")\n      FunctionBody span=5:15-7:3 is_vararg=false vararg_name=None\n        Param span=5:16-5:16 name=c\n        Block span=6:3-6:10\n          Return span=6:3-6:10\n            Name span=6:10-6:10 name=c\n",
    )
}

#[test]
fn tables_and_calls_snapshot() -> Result<(), String> {
    assert_snapshot(
        "return { [f(1)] = g; \"x\", x = 1, f(x), [30] = 23; 45 }, foo(bar), foo{1,2}, foo\"hi\"",
        "Chunk\n  Block span=1:1-1:83\n    Return span=1:1-1:83\n      Table span=1:8-1:54\n        TableConstructor span=1:8-1:54\n          KeyedField span=1:10-1:19\n            Call span=1:11-1:14\n              CallExpr span=1:11-1:14 method=None\n                Name span=1:11-1:11 name=f\n                Number span=1:13-1:13 lexeme=1\n            Name span=1:19-1:19 name=g\n          ArrayField span=1:22-1:24\n            String span=1:22-1:24 bytes=[120]\n          NamedField span=1:27-1:31 name=x\n            Number span=1:31-1:31 lexeme=1\n          ArrayField span=1:34-1:37\n            Call span=1:34-1:37\n              CallExpr span=1:34-1:37 method=None\n                Name span=1:34-1:34 name=f\n                Name span=1:36-1:36 name=x\n          KeyedField span=1:40-1:48\n            Number span=1:41-1:42 lexeme=30\n            Number span=1:47-1:48 lexeme=23\n          ArrayField span=1:51-1:52\n            Number span=1:51-1:52 lexeme=45\n      Call span=1:57-1:64\n        CallExpr span=1:57-1:64 method=None\n          Name span=1:57-1:59 name=foo\n          Name span=1:61-1:63 name=bar\n      Call span=1:67-1:74\n        CallExpr span=1:67-1:74 method=None\n          Name span=1:67-1:69 name=foo\n          Table span=1:70-1:74\n            TableConstructor span=1:70-1:74\n              ArrayField span=1:71-1:71\n                Number span=1:71-1:71 lexeme=1\n              ArrayField span=1:73-1:73\n                Number span=1:73-1:73 lexeme=2\n      Call span=1:77-1:83\n        CallExpr span=1:77-1:83 method=None\n          Name span=1:77-1:79 name=foo\n          String span=1:80-1:83 bytes=[104, 105]\n",
    )
}

#[test]
fn closures_and_varargs_snapshot() -> Result<(), String> {
    assert_snapshot(
        "local function outer(a, ...rest)\n  local b<const> = a\n  return function (x) return b, rest, x end\nend\n",
        "Chunk\n  Block span=1:1-4:3\n    LocalFunction span=1:1-4:3 name=outer\n      FunctionBody span=1:21-4:3 is_vararg=true vararg_name=Some(\"rest\")\n        Param span=1:22-1:22 name=a\n        Block span=2:3-3:43\n          LocalDecl span=2:3-2:20\n            AttributedName span=2:9-2:16 name=b\n              Attribute span=2:10-2:16 name=const\n            Name span=2:20-2:20 name=a\n          Return span=3:3-3:43\n            FunctionExpr span=3:10-3:43\n              FunctionBody span=3:19-3:43 is_vararg=false vararg_name=None\n                Param span=3:20-3:20 name=x\n                Block span=3:23-3:39\n                  Return span=3:23-3:39\n                    Name span=3:30-3:30 name=b\n                    Name span=3:33-3:36 name=rest\n                    Name span=3:39-3:39 name=x\n",
    )
}
