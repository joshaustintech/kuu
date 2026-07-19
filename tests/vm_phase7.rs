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
