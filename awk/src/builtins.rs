/// AWK Built-in Functions
use crate::value::Value;
use regex::Regex;
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

/// Get the builtin function registry
pub fn get_builtins() -> HashMap<&'static str, BuiltinFn> {
    let mut builtins: HashMap<&'static str, BuiltinFn> = HashMap::new();

    // String functions
    builtins.insert("length", builtin_length);
    builtins.insert("substr", builtin_substr);
    builtins.insert("index", builtin_index);
    builtins.insert("split", builtin_split);
    builtins.insert("sub", builtin_sub);
    builtins.insert("gsub", builtin_gsub);
    builtins.insert("match", builtin_match);
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

// String functions

fn builtin_length(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    let s = if args.is_empty() {
        ctx.record.to_string()
    } else {
        args[0].to_string()
    };
    Ok(Value::Number(s.len() as f64))
}

fn builtin_substr(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.is_empty() {
        return Err("substr requires at least 2 arguments".to_string());
    }

    let s = args[0].to_string();
    let chars: Vec<char> = s.chars().collect();

    // AWK uses 1-based indexing
    let start = if args.len() > 1 {
        (args[1].to_number() as i64 - 1).max(0) as usize
    } else {
        0
    };

    let len = if args.len() > 2 {
        args[2].to_number() as usize
    } else {
        chars.len().saturating_sub(start)
    };

    let end = (start + len).min(chars.len());
    let result: String = chars[start.min(chars.len())..end].iter().collect();

    Ok(Value::String(result))
}

fn builtin_index(args: &[Value], _ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("index requires 2 arguments".to_string());
    }

    let haystack = args[0].to_string();
    let needle = args[1].to_string();

    // AWK returns 1-based index, 0 if not found
    let pos = haystack.find(&needle).map(|i| i + 1).unwrap_or(0);

    Ok(Value::Number(pos as f64))
}

fn builtin_split(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("split requires at least 2 arguments".to_string());
    }

    let s = args[0].to_string();
    let _array_name = args[1].to_string();

    let sep = if args.len() > 2 {
        args[2].to_string()
    } else {
        ctx.variables
            .get("FS")
            .map(|v| v.to_string())
            .unwrap_or_else(|| " ".to_string())
    };

    let parts: Vec<&str> = if sep == " " {
        s.split_whitespace().collect()
    } else if sep.is_empty() {
        s.chars()
            .map(|c| {
                // This is a workaround - we need to return string slices
                // For single char split, we'll handle differently
                unsafe {
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                        &c as *const char as *const u8,
                        1,
                    ))
                }
            })
            .collect()
    } else {
        s.split(&sep).collect()
    };

    // Note: Array assignment would be handled by the interpreter
    // For now, return the count
    Ok(Value::Number(parts.len() as f64))
}

fn builtin_sub(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("sub requires at least 2 arguments".to_string());
    }

    let pattern = args[0].to_string();
    let replacement = args[1].to_string();

    // Default target is $0
    let target = if args.len() > 2 {
        args[2].to_string()
    } else {
        ctx.record.to_string()
    };

    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;

    // Handle & in replacement (represents matched text)
    let result = if let Some(m) = re.find(&target) {
        let repl = replacement.replace('&', m.as_str());
        let mut new_str = target.to_string();
        new_str.replace_range(m.start()..m.end(), &repl);
        new_str
    } else {
        target
    };

    // Return 1 if substitution made, 0 otherwise
    let made_sub = result != ctx.record;

    if args.len() <= 2 {
        // Update $0 - this would be handled by interpreter
    }

    Ok(Value::Number(if made_sub { 1.0 } else { 0.0 }))
}

fn builtin_gsub(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("gsub requires at least 2 arguments".to_string());
    }

    let pattern = args[0].to_string();
    let replacement = args[1].to_string();

    let target = if args.len() > 2 {
        args[2].to_string()
    } else {
        ctx.record.to_string()
    };

    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;

    let mut count = 0;
    let _result = re.replace_all(&target, |caps: &regex::Captures| {
        count += 1;
        replacement.replace('&', &caps[0])
    });

    Ok(Value::Number(count as f64))
}

fn builtin_match(args: &[Value], ctx: &mut BuiltinContext) -> BuiltinResult {
    if args.len() < 2 {
        return Err("match requires 2 arguments".to_string());
    }

    let s = args[0].to_string();
    let pattern = args[1].to_string();

    let re = Regex::new(&pattern).map_err(|e| e.to_string())?;

    if let Some(m) = re.find(&s) {
        // Set RSTART and RLENGTH
        ctx.variables
            .insert("RSTART".to_string(), Value::Number((m.start() + 1) as f64));
        ctx.variables
            .insert("RLENGTH".to_string(), Value::Number(m.len() as f64));
        Ok(Value::Number((m.start() + 1) as f64))
    } else {
        ctx.variables
            .insert("RSTART".to_string(), Value::Number(0.0));
        ctx.variables
            .insert("RLENGTH".to_string(), Value::Number(-1.0));
        Ok(Value::Number(0.0))
    }
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

/// Format string implementation (simplified printf)
pub fn format_string(format: &str, args: &[Value]) -> String {
    let mut result = String::new();
    let mut chars = format.chars().peekable();
    let mut arg_idx = 0;

    while let Some(c) = chars.next() {
        if c == '%' {
            if chars.peek() == Some(&'%') {
                chars.next();
                result.push('%');
                continue;
            }

            // Parse format specifier
            let mut width = String::new();
            let mut precision = String::new();
            let mut flags = String::new();
            let mut in_precision = false;

            // Flags
            while let Some(&c) = chars.peek() {
                if c == '-' || c == '+' || c == ' ' || c == '#' || c == '0' {
                    flags.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Width
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    width.push(chars.next().unwrap());
                } else if c == '.' {
                    chars.next();
                    in_precision = true;
                    break;
                } else {
                    break;
                }
            }

            // Precision
            if in_precision {
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        precision.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
            }

            // Conversion specifier
            if let Some(spec) = chars.next() {
                let val = if arg_idx < args.len() {
                    &args[arg_idx]
                } else {
                    &Value::Uninitialized
                };
                arg_idx += 1;

                let width: usize = width.parse().unwrap_or(0);
                let prec: usize = precision.parse().unwrap_or(6);
                let left_align = flags.contains('-');

                let formatted = match spec {
                    'd' | 'i' => {
                        let n = val.to_number() as i64;
                        format!("{}", n)
                    }
                    'o' => {
                        let n = val.to_number() as i64;
                        format!("{:o}", n)
                    }
                    'x' => {
                        let n = val.to_number() as i64;
                        format!("{:x}", n)
                    }
                    'X' => {
                        let n = val.to_number() as i64;
                        format!("{:X}", n)
                    }
                    'u' => {
                        let n = val.to_number() as u64;
                        format!("{}", n)
                    }
                    'c' => {
                        let s = val.to_string();
                        s.chars().next().map(|c| c.to_string()).unwrap_or_default()
                    }
                    's' => {
                        let s = val.to_string();
                        if !precision.is_empty() {
                            s.chars().take(prec).collect()
                        } else {
                            s
                        }
                    }
                    'e' => {
                        let n = val.to_number();
                        format!("{:.prec$e}", n, prec = prec)
                    }
                    'E' => {
                        let n = val.to_number();
                        format!("{:.prec$E}", n, prec = prec)
                    }
                    'f' => {
                        let n = val.to_number();
                        format!("{:.prec$}", n, prec = prec)
                    }
                    'g' => {
                        let n = val.to_number();
                        // Simplified g format
                        if n.abs() < 0.0001 || n.abs() >= 1e6 {
                            format!("{:.prec$e}", n, prec = prec)
                        } else {
                            format!("{:.prec$}", n, prec = prec)
                        }
                    }
                    'G' => {
                        let n = val.to_number();
                        if n.abs() < 0.0001 || n.abs() >= 1e6 {
                            format!("{:.prec$E}", n, prec = prec)
                        } else {
                            format!("{:.prec$}", n, prec = prec)
                        }
                    }
                    _ => format!("%{}", spec),
                };

                // Apply width
                if width > formatted.len() {
                    let padding = width - formatted.len();
                    if left_align {
                        result.push_str(&formatted);
                        result.push_str(&" ".repeat(padding));
                    } else {
                        let pad_char = if flags.contains('0') && !left_align {
                            '0'
                        } else {
                            ' '
                        };
                        result.push_str(&pad_char.to_string().repeat(padding));
                        result.push_str(&formatted);
                    }
                } else {
                    result.push_str(&formatted);
                }
            }
        } else if c == '\\' {
            // Handle escape sequences
            if let Some(next) = chars.next() {
                match next {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    _ => {
                        result.push('\\');
                        result.push(next);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
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
}
