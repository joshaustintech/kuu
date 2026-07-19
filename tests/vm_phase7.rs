mod support;

use support::{ExpectedRun, RunOptions, compare_run, fixture_path, run_binary, run_binary_with};

#[test]
fn global_declaration_without_assignment_preserves_existing_global()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "global none; global<const> assert, print; assert(true); print('ok')".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"ok\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn local_environment_redirects_global_accesses() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function f() local _ENV <const> = 11; X = 'hi' end; local ok, message = pcall(f); print(ok, string.find(message, 'number') ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn debug_upvalueid_preserves_shared_closure_identity() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local x = 1; local a = function() return x end; local b = function() return x end; print(debug.upvalueid(a, 1) == debug.upvalueid(b, 1))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn goto_uses_the_label_in_its_lexical_block() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local x = 13; do goto done; local a = 23; ::done:: end; do goto done; local b = 45; ::done:: end; print(x)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"13\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn goto_does_not_enter_a_sibling_if_branch() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function f() if true then goto done elseif false then ::done:: return 2 end; ::done:: return 1 end; print(f())".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"1\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn goto_closes_captured_locals_before_reentry() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local closures = {}; do ::again:: local x = #closures + 1; closures[#closures + 1] = function() return x end; if #closures < 2 then goto again end end; print(debug.upvalueid(closures[1], 1) ~= debug.upvalueid(closures[2], 1))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn global_keyword_can_be_a_user_mode_assignment_name() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "global = 1; print(global)".to_owned()],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"1\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn global_function_body_uses_its_declared_global() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "global print; local f = 20; do global function f(x) if x == 0 then return 1 end return 2 * f(x - 1) end; print(f(4)) end; print(_ENV.f(4), f)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"16\n16\t20\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn floating_division_by_zero_returns_ieee_values() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print((0 / 0) ~= (0 / 0), 1 / 0 > 0, -1 / 0 < 0)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttrue\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn floating_floor_division_by_zero_returns_infinity() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(1 // 0.0, -1 // 0.0, 1.0 // 0)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"inf\t-inf\tinf\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn integer_floor_division_keeps_integer_precision() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local min, max = math.mininteger, math.maxinteger; print((max - 1) // max == 0, (min + 1) // min == 0, min // (min + 1) == 1, min // -2 == 2^62)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttrue\ttrue\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn math_rounding_preserves_integer_inputs() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(math.type(math.floor(math.maxinteger)), math.type(math.ceil(math.mininteger)))"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"integer\tinteger\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn math_tointeger_parses_mininteger_text() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(math.tointeger(math.mininteger .. '') == math.mininteger)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn unary_minus_preserves_integer_type_and_wraps_minimum() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(math.type(-1), -math.mininteger == math.mininteger)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"integer\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn integer_float_comparison_keeps_maxinteger_precision() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local min, max = math.mininteger, math.maxinteger; print(max < min * -1.0, max <= min * -1.0)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn nan_ordering_comparisons_are_false() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local nan = 0 / 0; print(nan < 0, nan <= 0, 0 < nan, 0 <= nan)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\tfalse\tfalse\tfalse\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn nan_table_lookup_is_nil_but_writes_fail() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local nan, t = 0 / 0, {}; local ok = pcall(rawset, t, nan, 1); print(t[nan] == nil, ok)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\tfalse\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn math_random_seeded_float_uses_first_rng_word() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "math.randomseed(1007, 0); print(math.random() == 0x0.7a7040a5a323c9d6)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn numeric_for_stops_at_inclusive_bound() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local count = 0; for i = 0, 0 do count = count + 1; if count > 1 then break end end; print(count)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"1\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn numeric_for_uses_each_loop_binding() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local total = 0; for i = 1, 1 do total = total + i end; for i = 2, 2 do total = total + i end; print(total)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn math_fmod_preserves_float_result_for_float_input() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(math.type(math.fmod(-6, -6)), math.type(math.fmod(-6.0, -6)))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"integer\tfloat\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn goto_uses_nearest_same_named_label() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "do local n = 0; ::again:: n = n + 1; if n < 2 then goto again end end; do local n = 0; ::again:: n = n + 1; if n < 3 then goto again end; print(n) end"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tostring_keeps_large_integral_float_a_float() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local x = 2.0^56 + 8.0; print(tonumber(tostring(x)) == x)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn gsub_trims_decimal_padding_with_nongreedy_capture() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print((string.gsub('0009.6240', '^0*(%d.-%d)0*$', '%1')))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"9.624\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn string_pack_and_unpack_native_float() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local value, next = string.unpack('n', string.pack('n', 1.5)); print(value, next, string.packsize('n'))"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"1.5\t9\t8\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn require_returns_coroutine_module_table() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(require('coroutine') == coroutine, type(coroutine))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttable\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn table_concat_joins_range_with_separator() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(table.concat({'a', 2, 'c'}, ':'), table.concat({'a', 'b', 'c'}, '', 2, 3))"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"a:2:c\tbc\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn require_lists_path_and_cpath_candidates() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "package.path = '?.lua;?/?'; package.cpath = '?.so;?/init'; local ok, message = pcall(require, 'XXX'); print(ok, message)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\tmodule 'XXX' not found:\n\tno field package.preload['XXX']\n\tno file 'XXX.lua'\n\tno file 'XXX/XXX'\n\tno file 'XXX.so'\n\tno file 'XXX/init'\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn pcall_returns_raw_runtime_message() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local ok, message = pcall(require, 'missing-module'); print(ok, string.find(message, 'runtime error') == nil)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn portable_mode_and_table_unpack_are_available() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local a, b = table.unpack({'a', 'b'}); print(_port, a, b)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ta\tb\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn literal_integer_floor_division_by_zero_errors_when_called()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local f = assert(load('return 2 // 0')); local ok, err = pcall(f); print(ok, string.find(err, 'divide by zero') ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn fractional_bitwise_operand_reports_missing_integer_representation()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local f = assert(load('return 2.3 >> 0')); local ok, err = pcall(f); print(ok, string.find(err, 'number.* has no integer representation') ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn strings_expose_string_library_methods_and_missing_fields_are_nil()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local text = 'ok'; print(text:format('%s'), text.missing == nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"ok\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn bitwise_math_huge_error_identifies_the_huge_field() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local f = assert(load('return math.huge << 1')); local ok, err = pcall(f); print(ok, string.find(err, \"field 'huge'\") ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn infinite_bitwise_operand_reports_missing_integer_representation()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local function f2i(value) return value | value end; local ok, err = pcall(f2i, math.huge); print(ok, string.find(err, 'number.* has no integer representation') ~= nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"false\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn arithmetic_coerces_trimmed_decimal_and_hex_strings() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(' 3e0 ' + 2, ' -0xa ' + 1)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"5\t-9\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn gsub_replaces_the_final_digit_with_a_function_result() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local value, count = string.gsub('18', '%d$', function(digit) return string.char(string.byte(digit) + 1) end); print(value, count)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"19\t1\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tonumber_wraps_oversized_hex_integer_text() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(tonumber('0x1' .. string.rep('0', 30)))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"0\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tonumber_parses_long_hex_float_text() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(tonumber('0x' .. string.rep('f', 150) .. '.0') == 2.0^600 - 1)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tonumber_scales_huge_hex_mantissa_before_negative_exponent()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(tonumber('0xe03' .. string.rep('0', 1000) .. 'p-4000'))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"3587\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tonumber_scales_long_hex_fraction_before_positive_exponent()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(tonumber('0x.' .. string.rep('0', 1000) .. '74p4004') == 0x7.4)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tonumber_rejects_infinity_and_nan_text() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(tonumber('inf') == nil, tonumber('Nan') == nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tonumber_rejects_hex_float_without_exponent_digits() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(tonumber('-0xaaP ') == nil)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn arithmetic_coerces_hex_float_strings() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(1 - '0x.00000001' == 0x.FfffFFFF)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn integer_modulo_uses_lua_floor_semantics() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "print(-4 % 3, 4 % -3, math.type(-4 % 3))".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"2\t-2\tinteger\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn floating_modulo_uses_lua_floor_semantics() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec!["-e".to_owned(), "print(-4.0 % 3, 4 % -3.0)".to_owned()],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"2\t-2\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn floating_modulo_keeps_high_power_precision_and_nan_zero_divisor()
-> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local nan = 0.0 % 0; print(2^60 % 3, 2^61 % 3, nan ~= nan)".to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"1\t2\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn frexp_preserves_negative_mantissa_sign() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary_with(RunOptions {
        args: vec![
            "-e".to_owned(),
            "local m, p = math.frexp(-math.pi); print(m < 0, math.ldexp(m, p) == -math.pi)"
                .to_owned(),
        ],
        ..RunOptions::default()
    })?;
    let expected = ExpectedRun {
        stdout: b"true\ttrue\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn prints_the_hello_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("hello.lua"))?;
    let expected = ExpectedRun {
        stdout: b"hello\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn tables_globals_and_identity_work() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_tables.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t42\ttrue\tok\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn closures_and_closed_upvalues_survive_the_outer_return() -> Result<(), Box<dyn std::error::Error>>
{
    let actual = run_binary(&fixture_path("phase7_upvalues.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t2\t3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn varargs_flow_through_lua_calls() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_varargs.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t2\t3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn forwarded_varargs_reach_the_next_call() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_vararg_forward.lua"))?;
    let expected = ExpectedRun {
        stdout: b"1\t2\t3\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}

#[test]
fn recursion_and_tail_calls_use_vm_frames() -> Result<(), Box<dyn std::error::Error>> {
    let actual = run_binary(&fixture_path("phase7_recursion.lua"))?;
    let expected = ExpectedRun {
        stdout: b"15\n",
        stderr: b"",
        exit_code: Some(0),
    };
    compare_run(&actual, &expected)?;
    Ok(())
}
