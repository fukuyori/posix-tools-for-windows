/// AWK Values and Variables
use std::collections::HashMap;

/// AWK value - can be number, string, or uninitialized
#[derive(Debug, Clone)]
pub enum Value {
    Uninitialized,
    Number(f64),
    String(String),
    /// Numeric string (string that looks like a number)
    NumericString(String, f64),
}

impl Default for Value {
    fn default() -> Self {
        Value::Uninitialized
    }
}

impl Value {
    /// Convert to number
    pub fn to_number(&self) -> f64 {
        match self {
            Value::Uninitialized => 0.0,
            Value::Number(n) => *n,
            Value::String(s) => parse_number(s),
            Value::NumericString(_, n) => *n,
        }
    }

    /// Convert to string
    pub fn to_string(&self) -> String {
        match self {
            Value::Uninitialized => String::new(),
            Value::Number(n) => format_number(*n),
            Value::String(s) => s.clone(),
            Value::NumericString(s, _) => s.clone(),
        }
    }

    /// Convert to string with OFMT format
    pub fn to_string_with_ofmt(&self, ofmt: &str) -> String {
        match self {
            Value::Uninitialized => String::new(),
            Value::Number(n) => format_number_with_fmt(*n, ofmt),
            Value::String(s) => s.clone(),
            Value::NumericString(s, _) => s.clone(),
        }
    }

    /// Convert to boolean
    pub fn to_bool(&self) -> bool {
        match self {
            Value::Uninitialized => false,
            Value::Number(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::NumericString(s, n) => *n != 0.0 || !s.is_empty(),
        }
    }

    /// Check if value is numeric
    pub fn is_numeric(&self) -> bool {
        matches!(self, Value::Number(_) | Value::NumericString(_, _))
    }

    /// Create from string, detecting numeric strings
    pub fn from_string(s: String) -> Self {
        let trimmed = s.trim();
        if let Some(n) = try_parse_number(trimmed) {
            Value::NumericString(s, n)
        } else {
            Value::String(s)
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Number(n as f64)
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

/// Parse a string as a number (AWK style)
fn parse_number(s: &str) -> f64 {
    try_parse_number(s).unwrap_or(0.0)
}

/// Try to parse a string as a number
fn try_parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0.0);
    }

    // Find the numeric prefix
    let mut end = 0;
    let chars: Vec<char> = s.chars().collect();

    // Optional sign
    if end < chars.len() && (chars[end] == '+' || chars[end] == '-') {
        end += 1;
    }

    // Digits before decimal
    let mut has_digits = false;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
        has_digits = true;
    }

    // Decimal point
    if end < chars.len() && chars[end] == '.' {
        end += 1;
        // Digits after decimal
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
            has_digits = true;
        }
    }

    // Exponent
    if end < chars.len() && (chars[end] == 'e' || chars[end] == 'E') {
        let exp_start = end;
        end += 1;
        if end < chars.len() && (chars[end] == '+' || chars[end] == '-') {
            end += 1;
        }
        let mut has_exp_digits = false;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
            has_exp_digits = true;
        }
        if !has_exp_digits {
            end = exp_start; // Rollback
        }
    }

    if !has_digits {
        return None;
    }

    let num_str: String = chars[..end].iter().collect();
    num_str.parse().ok()
}

/// Format a number for output
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        // Use %.6g style formatting
        let formatted = format!("{:.6}", n);
        // Remove trailing zeros after decimal point
        let formatted = formatted.trim_end_matches('0');
        let formatted = formatted.trim_end_matches('.');
        formatted.to_string()
    }
}

/// Format a number with a specific format
fn format_number_with_fmt(n: f64, fmt: &str) -> String {
    if fmt == "%.6g" {
        return format_number(n);
    }

    let Some(stripped) = fmt.strip_prefix('%') else {
        return format_number(n);
    };
    let Some(spec) = stripped.chars().last() else {
        return format_number(n);
    };

    let body = &stripped[..stripped.len().saturating_sub(spec.len_utf8())];
    let precision = body
        .strip_prefix('.')
        .and_then(|p| p.parse::<usize>().ok())
        .unwrap_or(6);

    match spec {
        'f' | 'F' => format!("{:.precision$}", n, precision = precision),
        'e' => format!("{:.precision$e}", n, precision = precision),
        'E' => format!("{:.precision$E}", n, precision = precision),
        'g' => format_general(n, precision, false),
        'G' => format_general(n, precision, true),
        _ => format_number(n),
    }
}

fn format_general(n: f64, precision: usize, upper: bool) -> String {
    let precision = precision.max(1);
    let abs = n.abs();
    let use_exp = (abs != 0.0 && abs < 0.0001) || abs >= 10f64.powi(precision as i32);

    let raw = if use_exp {
        if upper {
            format!("{:.prec$E}", n, prec = precision.saturating_sub(1))
        } else {
            format!("{:.prec$e}", n, prec = precision.saturating_sub(1))
        }
    } else {
        format!("{:.prec$}", n, prec = precision.saturating_sub(1))
    };

    trim_float_string(&raw)
}

fn trim_float_string(s: &str) -> String {
    if let Some(exp_pos) = s.find(['e', 'E']) {
        let (mantissa, exponent) = s.split_at(exp_pos);
        format!("{}{}", trim_decimal_part(mantissa), exponent)
    } else {
        trim_decimal_part(s)
    }
}

fn trim_decimal_part(s: &str) -> String {
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Compare two values
pub fn compare_values(left: &Value, right: &Value) -> std::cmp::Ordering {
    // If both are numeric, compare as numbers
    if left.is_numeric() && right.is_numeric() {
        let l = left.to_number();
        let r = right.to_number();
        l.partial_cmp(&r).unwrap_or(std::cmp::Ordering::Equal)
    } else {
        // Otherwise compare as strings
        let l = left.to_string();
        let r = right.to_string();
        l.cmp(&r)
    }
}

/// Variable storage
#[derive(Debug, Default)]
pub struct Variables {
    /// Scalar variables
    pub scalars: HashMap<String, Value>,
    /// Array variables
    pub arrays: HashMap<String, HashMap<String, Value>>,
}

impl Variables {
    pub fn new() -> Self {
        Variables {
            scalars: HashMap::new(),
            arrays: HashMap::new(),
        }
    }

    /// Get a scalar variable
    pub fn get(&self, name: &str) -> Value {
        self.scalars
            .get(name)
            .cloned()
            .unwrap_or(Value::Uninitialized)
    }

    /// Set a scalar variable
    pub fn set(&mut self, name: &str, value: Value) {
        self.scalars.insert(name.to_string(), value);
    }

    /// Get an array element
    pub fn get_array(&self, name: &str, key: &str) -> Value {
        self.arrays
            .get(name)
            .and_then(|arr| arr.get(key))
            .cloned()
            .unwrap_or(Value::Uninitialized)
    }

    /// Set an array element
    pub fn set_array(&mut self, name: &str, key: &str, value: Value) {
        self.arrays
            .entry(name.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Check if array element exists
    pub fn has_array_key(&self, name: &str, key: &str) -> bool {
        self.arrays
            .get(name)
            .is_some_and(|arr| arr.contains_key(key))
    }

    /// Delete array element
    pub fn delete_array(&mut self, name: &str, key: &str) {
        if let Some(arr) = self.arrays.get_mut(name) {
            arr.remove(key);
        }
    }

    /// Get array keys
    pub fn array_keys(&self, name: &str) -> Vec<String> {
        self.arrays
            .get(name)
            .map(|arr| arr.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_conversion() {
        let v = Value::Number(42.0);
        assert_eq!(v.to_number(), 42.0);
        assert_eq!(v.to_string(), "42");
        assert!(v.to_bool());

        let v = Value::String("123".to_string());
        assert_eq!(v.to_number(), 123.0);

        let v = Value::String("hello".to_string());
        assert_eq!(v.to_number(), 0.0);
    }

    #[test]
    fn test_numeric_string() {
        let v = Value::from_string("  42.5  ".to_string());
        assert!(v.is_numeric());
        assert_eq!(v.to_number(), 42.5);
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(3.14159), "3.14159");
        assert_eq!(format_number(0.0), "0");
    }

    #[test]
    fn test_format_number_with_ofmt() {
        assert_eq!(format_number_with_fmt(3.5, "%.2f"), "3.50");
        assert_eq!(format_number_with_fmt(12.0, "%.3g"), "12");
        assert_eq!(format_number_with_fmt(0.00123, "%.2e"), "1.23e-3");
    }
}
