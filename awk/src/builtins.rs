/// AWK Built-in Functions
use crate::value::Value;
use std::collections::HashMap;

/// Result of a builtin function
pub type BuiltinResult = Result<Value, String>;

/// Builtin function signature
pub type BuiltinFn = fn(&[Value], &mut BuiltinContext) -> BuiltinResult;

/// Context for builtin functions that need special access
#[allow(dead_code)]
pub struct BuiltinContext<'a> {
    pub record: &'a str,
    pub fields: &'a mut Vec<String>,
    pub variables: &'a mut HashMap<String, Value>,
    pub subsep: &'a str,
    pub rng_state: &'a mut u64,
}

/// Get the builtin function registry.
/// Note: `split`, `sub`, `gsub`, `match`, `close`, `fflush`, and `system`
/// are handled directly by the interpreter because they need lvalue or
/// I/O access; they are not in this table.
pub fn get_builtins() -> HashMap<&'static str, BuiltinFn> {
    let mut builtins: HashMap<&'static str, BuiltinFn> = HashMap::new();

    // String functions
    builtins.insert("length", builtin_length);
    builtins.insert("substr", builtin_substr);
    builtins.insert("index", builtin_index);
    builtins.insert("sprintf", builtin_sprintf);
    builtins.insert("tolower", builtin_tolower);
    builtins.insert("toupper", builtin_toupper);

    // Math functions
    builtins.insert("sin", builtin_sin);
    builtins.insert("cos", builtin_cos);
    builtins.insert("atan2", builtin_atan2);
    builtins.insert("exp", builtin_exp);
    builtins.insert("log", builtin_log);
    builtins.insert("sqrt", builtin_sqrt);
    builtins.insert("int", builtin_int);
    builtins.insert("rand", builtin_rand);
    builtins.insert("srand", builtin_srand);

    builtins
}

/// Names that the parser should treat as callable builtins even when a
/// space separates the name from `(`.
pub fn is_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "length"
            | "substr"
            | "index"
            | "split"
            | "sub"
            | "gsub"
            | "match"
            | "sprintf"
            | "tolower"
            | "toupper"
            | "sin"
            | "cos"
            | "atan2"
            | "exp"
            | "log"
            | "sqrt"
            | "int"
            | "rand"
            | "srand"
            | "close"
            | "fflush"
            | "system"
    )
}

// String functions

fn builtin_length(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    let s = if args.is_empty() {
        ctx.record.to_string()
    } else {
        args[0].to_string()
    };
    Ok(Value::Number(s.chars().count() as f64))
}

/// substr(s, m[, n]) with POSIX clamping semantics:
/// characters before position 1 count against n; out-of-range yields "".
fn builtin_substr(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("substr requires at least 2 arguments".to_string());
    }

    let s = args[0].to_string();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;

    let m = args[1].to_number().trunc() as i64;
    // Exclusive end position (1-based): m + n, or end of string
    let end = if args.len() > 2 {
        let n = args[2].to_number().trunc() as i64;
        m.saturating_add(n)
    } else {
        len + 1
    };

    let start = m.max(1);
    let end = end.min(len + 1);
    if start > len || end <= start {
        return Ok(Value::String(String::new()));
    }

    let result: String = chars[(start - 1) as usize..(end - 1) as usize]
        .iter()
        .collect();
    Ok(Value::String(result))
}

fn builtin_index(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("index requires 2 arguments".to_string());
    }

    let haystack = args[0].to_string();
    let needle = args[1].to_string();

    if needle.is_empty() {
        return Ok(Value::Number(0.0));
    }

    // AWK returns 1-based character index, 0 if not found
    let pos = haystack
        .find(&needle)
        .map(|i| haystack[..i].chars().count() + 1)
        .unwrap_or(0);

    Ok(Value::Number(pos as f64))
}

fn builtin_sprintf(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("sprintf requires at least 1 argument".to_string());
    }

    let format = args[0].to_string();
    let result = format_string(&format, &args[1..]);

    Ok(Value::String(result))
}

fn builtin_tolower(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("tolower requires 1 argument".to_string());
    }
    Ok(Value::String(args[0].to_string().to_lowercase()))
}

fn builtin_toupper(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("toupper requires 1 argument".to_string());
    }
    Ok(Value::String(args[0].to_string().to_uppercase()))
}

// Math functions

fn builtin_sin(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("sin requires 1 argument".to_string());
    }
    Ok(Value::Number(args[0].to_number().sin()))
}

fn builtin_cos(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("cos requires 1 argument".to_string());
    }
    Ok(Value::Number(args[0].to_number().cos()))
}

fn builtin_atan2(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("atan2 requires 2 arguments".to_string());
    }
    let y = args[0].to_number();
    let x = args[1].to_number();
    Ok(Value::Number(y.atan2(x)))
}

fn builtin_exp(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("exp requires 1 argument".to_string());
    }
    Ok(Value::Number(args[0].to_number().exp()))
}

fn builtin_log(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("log requires 1 argument".to_string());
    }
    Ok(Value::Number(args[0].to_number().ln()))
}

fn builtin_sqrt(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("sqrt requires 1 argument".to_string());
    }
    Ok(Value::Number(args[0].to_number().sqrt()))
}

fn builtin_int(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("int requires 1 argument".to_string());
    }
    Ok(Value::Number(args[0].to_number().trunc()))
}

fn builtin_rand(_args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    // Simple LCG random number generator
    *ctx.rng_state = ctx.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
    let val = ((*ctx.rng_state >> 16) & 0x7fff) as f64 / 32768.0;
    Ok(Value::Number(val))
}

fn builtin_srand(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    let old_seed = *ctx.rng_state;

    *ctx.rng_state = if args.is_empty() {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    } else {
        args[0].to_number() as u64
    };

    Ok(Value::Number(old_seed as f64))
}

// ---------------------------------------------------------------------------
// printf-style formatting
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
struct FmtFlags {
    minus: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
}

/// Format string implementation (C printf subset per POSIX awk):
/// flags `- + space # 0`, width (including `*`), precision (including `.*`),
/// conversions `d i o u x X c s e E f F g G %`.
/// Escape sequences are NOT processed here — string literals already had
/// them expanded by the lexer, and dynamic strings must stay untouched.
pub fn format_string(format: &str, args: &[Value]) -> String {
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    let mut arg_idx = 0usize;

    fn take_arg(args: &[Value], idx: &mut usize) -> Value {
        let v = args.get(*idx).cloned().unwrap_or(Value::Uninitialized);
        *idx += 1;
        v
    }

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }

        // Flags
        let mut flags = FmtFlags::default();
        while let Some(&f) = chars.peek() {
            match f {
                '-' => flags.minus = true,
                '+' => flags.plus = true,
                ' ' => flags.space = true,
                '#' => flags.alt = true,
                '0' => flags.zero = true,
                _ => break,
            }
            chars.next();
        }

        // Width (digits or '*')
        let mut width: Option<usize> = None;
        if chars.peek() == Some(&'*') {
            chars.next();
            let w = take_arg(args, &mut arg_idx).to_number() as i64;
            if w < 0 {
                flags.minus = true;
                width = Some(w.unsigned_abs() as usize);
            } else {
                width = Some(w as usize);
            }
        } else {
            let mut w = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    w.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            if !w.is_empty() {
                width = w.parse().ok();
            }
        }

        // Precision ('.' then digits or '*')
        let mut precision: Option<usize> = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            if chars.peek() == Some(&'*') {
                chars.next();
                let p = take_arg(args, &mut arg_idx).to_number() as i64;
                if p >= 0 {
                    precision = Some(p as usize);
                }
            } else {
                let mut p = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        p.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                precision = Some(p.parse().unwrap_or(0));
            }
        }

        // Conversion specifier
        let Some(spec) = chars.next() else {
            out.push('%');
            break;
        };

        let body = match spec {
            'd' | 'i' => {
                let val = take_arg(args, &mut arg_idx);
                format_signed_int(val.to_number(), precision, flags)
            }
            'o' | 'u' | 'x' | 'X' => {
                let val = take_arg(args, &mut arg_idx);
                format_unsigned_int(val.to_number(), spec, precision, flags)
            }
            'c' => {
                let val = take_arg(args, &mut arg_idx);
                format_char(&val)
            }
            's' => {
                let val = take_arg(args, &mut arg_idx);
                let s = val.to_string();
                if let Some(p) = precision {
                    s.chars().take(p).collect()
                } else {
                    s
                }
            }
            'e' | 'E' => {
                let val = take_arg(args, &mut arg_idx);
                format_float_sign(val.to_number(), flags, |n| {
                    format_float_exp(n, precision.unwrap_or(6), spec == 'E', flags.alt)
                })
            }
            'f' | 'F' => {
                let val = take_arg(args, &mut arg_idx);
                format_float_sign(val.to_number(), flags, |n| {
                    format_float_fixed(n, precision.unwrap_or(6), flags.alt)
                })
            }
            'g' | 'G' => {
                let val = take_arg(args, &mut arg_idx);
                format_float_sign(val.to_number(), flags, |n| {
                    format_float_general(n, precision.unwrap_or(6), spec == 'G', flags.alt)
                })
            }
            other => {
                // Unknown conversion: emit it literally
                out.push('%');
                out.push(other);
                continue;
            }
        };

        out.push_str(&pad_field(&body, width, spec, precision, flags));
    }

    out
}

fn format_char(val: &Value) -> String {
    match val {
        Value::Number(n) | Value::NumericString(_, n) => {
            let code = *n as i64;
            if code <= 0 {
                String::new()
            } else {
                char::from_u32(code as u32)
                    .map(|c| c.to_string())
                    .unwrap_or_default()
            }
        }
        _ => val
            .to_string()
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default(),
    }
}

fn format_signed_int(n: f64, precision: Option<usize>, flags: FmtFlags) -> String {
    let i = if n.is_nan() {
        0
    } else {
        n.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64
    };
    let mut digits = format!("{}", i.unsigned_abs());
    if let Some(p) = precision {
        if p == 0 && i == 0 {
            digits.clear();
        } else if digits.len() < p {
            digits = format!("{}{}", "0".repeat(p - digits.len()), digits);
        }
    }
    let sign = if i < 0 {
        "-"
    } else if flags.plus {
        "+"
    } else if flags.space {
        " "
    } else {
        ""
    };
    format!("{}{}", sign, digits)
}

fn format_unsigned_int(n: f64, spec: char, precision: Option<usize>, flags: FmtFlags) -> String {
    // Negative values wrap like a 64-bit two's-complement unsigned int
    let u = if n.is_nan() {
        0u64
    } else if n < 0.0 {
        n.trunc().clamp(i64::MIN as f64, i64::MAX as f64) as i64 as u64
    } else {
        n.trunc().min(u64::MAX as f64) as u64
    };
    let mut digits = match spec {
        'o' => format!("{:o}", u),
        'x' => format!("{:x}", u),
        'X' => format!("{:X}", u),
        _ => format!("{}", u),
    };
    if let Some(p) = precision {
        if p == 0 && u == 0 {
            digits.clear();
        } else if digits.len() < p {
            digits = format!("{}{}", "0".repeat(p - digits.len()), digits);
        }
    }
    if flags.alt && u != 0 {
        match spec {
            'o' if !digits.starts_with('0') => digits = format!("0{}", digits),
            'x' => digits = format!("0x{}", digits),
            'X' => digits = format!("0X{}", digits),
            _ => {}
        }
    }
    digits
}

/// Apply sign handling for float conversions: format |n| via `f`,
/// then prepend '-', '+', or ' ' as appropriate.
fn format_float_sign<F: Fn(f64) -> String>(n: f64, flags: FmtFlags, f: F) -> String {
    let neg = n < 0.0 || (n == 0.0 && n.is_sign_negative());
    let body = if n.is_nan() {
        "nan".to_string()
    } else if n.is_infinite() {
        "inf".to_string()
    } else {
        f(n.abs())
    };
    let sign = if neg {
        "-"
    } else if flags.plus {
        "+"
    } else if flags.space {
        " "
    } else {
        ""
    };
    format!("{}{}", sign, body)
}

/// %f for a non-negative finite value
fn format_float_fixed(n: f64, precision: usize, alt: bool) -> String {
    let mut s = format!("{:.prec$}", n, prec = precision);
    if precision == 0 && alt {
        s.push('.');
    }
    s
}

/// %e / %E for a non-negative finite value: C-style exponent (e+NN)
fn format_float_exp(n: f64, precision: usize, upper: bool, alt: bool) -> String {
    let raw = format!("{:.prec$e}", n, prec = precision);
    let (mut mantissa, exp) = split_exponent(&raw);
    if precision == 0 && alt && !mantissa.contains('.') {
        mantissa.push('.');
    }
    let e = if upper { 'E' } else { 'e' };
    let sign = if exp < 0 { '-' } else { '+' };
    format!("{}{}{}{:02}", mantissa, e, sign, exp.abs())
}

/// %g / %G for a non-negative finite value. `precision` is the number of
/// significant digits. Also used for OFMT/CONVFMT-style conversion.
pub fn format_float_general(n: f64, precision: usize, upper: bool, alt: bool) -> String {
    let p = precision.max(1);
    if n == 0.0 {
        return "0".to_string();
    }

    // Determine the decimal exponent after rounding to p significant digits
    let rounded = format!("{:.prec$e}", n, prec = p - 1);
    let (mantissa, exp) = split_exponent(&rounded);

    if exp < -4 || exp >= p as i32 {
        // Exponential form
        let mantissa = if alt {
            mantissa
        } else {
            trim_trailing_zeros(&mantissa)
        };
        let e = if upper { 'E' } else { 'e' };
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}{}{}{:02}", mantissa, e, sign, exp.abs())
    } else {
        // Fixed form with p significant digits
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        let s = format!("{:.prec$}", n, prec = decimals);
        if alt {
            s
        } else {
            trim_trailing_zeros(&s)
        }
    }
}

/// Split Rust's `{:e}` output ("1.234e5") into (mantissa, exponent)
fn split_exponent(s: &str) -> (String, i32) {
    match s.find(['e', 'E']) {
        Some(pos) => {
            let exp = s[pos + 1..].parse::<i32>().unwrap_or(0);
            (s[..pos].to_string(), exp)
        }
        None => (s.to_string(), 0),
    }
}

fn trim_trailing_zeros(s: &str) -> String {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// Pad a formatted field to `width`.
fn pad_field(
    body: &str,
    width: Option<usize>,
    spec: char,
    precision: Option<usize>,
    flags: FmtFlags,
) -> String {
    let Some(w) = width else {
        return body.to_string();
    };
    let len = body.chars().count();
    if len >= w {
        return body.to_string();
    }
    let pad = w - len;

    if flags.minus {
        return format!("{}{}", body, " ".repeat(pad));
    }

    // '0' flag: pad with zeros after the sign for numeric conversions,
    // but not for integers with an explicit precision, and not for %c/%s.
    let int_spec = matches!(spec, 'd' | 'i' | 'o' | 'u' | 'x' | 'X');
    let numeric = int_spec || matches!(spec, 'e' | 'E' | 'f' | 'F' | 'g' | 'G');
    if flags.zero && numeric && !(int_spec && precision.is_some()) && !body.ends_with("inf") && !body.ends_with("nan") {
        let (sign, rest) = match body.chars().next() {
            Some(c @ ('-' | '+' | ' ')) => (c.to_string(), &body[c.len_utf8()..]),
            _ => (String::new(), body),
        };
        return format!("{}{}{}", sign, "0".repeat(pad), rest);
    }

    format!("{}{}", " ".repeat(pad), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sprintf() {
        let result = format_string(
            "Hello %s, you are %d years old",
            &[Value::String("World".to_string()), Value::Number(42.0)],
        );
        assert_eq!(result, "Hello World, you are 42 years old");
    }

    #[test]
    fn test_sprintf_width() {
        let result = format_string("[%10s]", &[Value::String("hi".to_string())]);
        assert_eq!(result, "[        hi]");

        let result = format_string("[%-10s]", &[Value::String("hi".to_string())]);
        assert_eq!(result, "[hi        ]");
    }

    #[test]
    fn test_printf_flags() {
        assert_eq!(format_string("%+d", &[Value::Number(42.0)]), "+42");
        assert_eq!(format_string("% d", &[Value::Number(42.0)]), " 42");
        assert_eq!(format_string("%05d", &[Value::Number(42.0)]), "00042");
        assert_eq!(format_string("%05d", &[Value::Number(-42.0)]), "-0042");
        assert_eq!(format_string("%#o", &[Value::Number(8.0)]), "010");
        assert_eq!(format_string("%#x", &[Value::Number(255.0)]), "0xff");
    }

    #[test]
    fn test_printf_dynamic_width_precision() {
        assert_eq!(
            format_string("[%*d]", &[Value::Number(5.0), Value::Number(42.0)]),
            "[   42]"
        );
        assert_eq!(
            format_string(
                "[%*.*f]",
                &[Value::Number(8.0), Value::Number(2.0), Value::Number(3.14159)]
            ),
            "[    3.14]"
        );
    }

    #[test]
    fn test_printf_char() {
        assert_eq!(format_string("%c", &[Value::Number(65.0)]), "A");
        assert_eq!(
            format_string("%c", &[Value::String("hello".to_string())]),
            "h"
        );
        assert_eq!(
            format_string("%c", &[Value::NumericString("65".to_string(), 65.0)]),
            "A"
        );
    }

    #[test]
    fn test_printf_exponent_format() {
        assert_eq!(
            format_string("%e", &[Value::Number(12345.6789)]),
            "1.234568e+04"
        );
        assert_eq!(format_string("%E", &[Value::Number(0.5)]), "5.000000E-01");
    }

    #[test]
    fn test_printf_general_format() {
        assert_eq!(format_string("%g", &[Value::Number(0.00001)]), "1e-05");
        assert_eq!(
            format_string("%g", &[Value::Number(123456789.0)]),
            "1.23457e+08"
        );
        assert_eq!(format_string("%g", &[Value::Number(100.0)]), "100");
        assert_eq!(format_string("%.3g", &[Value::Number(3.14159)]), "3.14");
    }
}
