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
            // A numeric string is treated as a number: "0" is false
            Value::NumericString(_, n) => *n != 0.0,
        }
    }

    /// Check if value is numeric
    #[allow(dead_code)]
    pub fn is_numeric(&self) -> bool {
        matches!(self, Value::Number(_) | Value::NumericString(_, _))
    }

    /// Create from string, detecting numeric strings.
    /// Per POSIX a numeric string must consist entirely of a number
    /// (with optional surrounding whitespace); "12abc" is NOT numeric.
    pub fn from_string(s: String) -> Self {
        if let Some(n) = parse_full_number(s.trim()) {
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

/// Parse a string as a number (AWK style: numeric prefix, 0 if none)
fn parse_number(s: &str) -> f64 {
    let (n, _) = parse_number_prefix(s);
    n
}

/// Parse the numeric prefix of a string.
/// Returns (value, chars consumed of the trimmed string).
fn parse_number_prefix(s: &str) -> (f64, usize) {
    let s = s.trim();
    if s.is_empty() {
        return (0.0, 0);
    }

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
    if has_digits && end < chars.len() && (chars[end] == 'e' || chars[end] == 'E') {
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
        return (0.0, 0);
    }

    let num_str: String = chars[..end].iter().collect();
    (num_str.parse().unwrap_or(0.0), end)
}

/// Parse a string that must be entirely a number (no trailing junk).
/// Used for numeric-string detection. Empty strings are not numeric.
fn parse_full_number(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let (n, consumed) = parse_number_prefix(s);
    if consumed == s.chars().count() {
        Some(n)
    } else {
        None
    }
}

/// Format a number for output.
/// Integral values are printed as integers (POSIX: "%d" conversion);
/// other values use %.6g (the default CONVFMT/OFMT).
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "nan".to_string();
    }
    if n.is_infinite() {
        return if n < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    if n.fract() == 0.0 && n.abs() < 9.2e18 {
        format!("{}", n as i64)
    } else {
        crate::builtins::format_float_general(n, 6, false, false)
    }
}

/// Format a number with a specific format (OFMT/CONVFMT).
/// Per POSIX, integral values always use "%d"-style conversion.
pub fn format_number_with_fmt(n: f64, fmt: &str) -> String {
    if n.fract() == 0.0 || !n.is_finite() {
        return format_number(n);
    }
    crate::builtins::format_string(fmt, &[Value::Number(n)])
}

/// Compare two values per POSIX comparison rules.
/// Uninitialized values act as both 0 and "" — they compare numerically
/// against numbers/numeric strings and as "" against plain strings.
/// Returns None when the comparison is unordered (NaN involved).
pub fn compare_values(left: &Value, right: &Value) -> Option<std::cmp::Ordering> {
    let numeric = |v: &Value| {
        matches!(
            v,
            Value::Number(_) | Value::NumericString(_, _) | Value::Uninitialized
        )
    };
    if numeric(left) && numeric(right) {
        left.to_number().partial_cmp(&right.to_number())
    } else {
        // Otherwise compare as strings
        let l = left.to_string();
        let r = right.to_string();
        Some(l.cmp(&r))
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
        assert_eq!(format_number_with_fmt(0.00123, "%.2e"), "1.23e-03");
    }

    #[test]
    fn test_numeric_string_requires_full_number() {
        assert!(!Value::from_string("12abc".to_string()).is_numeric());
        assert!(!Value::from_string("".to_string()).is_numeric());
        assert!(Value::from_string(" 1e3 ".to_string()).is_numeric());
    }

    #[test]
    fn test_uninitialized_compares_as_zero_and_empty() {
        let u = Value::Uninitialized;
        assert_eq!(
            compare_values(&u, &Value::Number(0.0)),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            compare_values(&u, &Value::String(String::new())),
            Some(std::cmp::Ordering::Equal)
        );
    }

    #[test]
    fn test_nan_comparison_is_unordered() {
        let nan = Value::Number(f64::NAN);
        assert_eq!(compare_values(&nan, &nan), None);
    }
}
