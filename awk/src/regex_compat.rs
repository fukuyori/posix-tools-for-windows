use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub struct PosixRegex {
    ast: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
enum Expr {
    Empty,
    Literal(char),
    Dot,
    Class(CharClass),
    AnchorStart,
    AnchorEnd,
    Concat(Vec<Expr>),
    Alternate(Vec<Expr>),
    Repeat {
        expr: Box<Expr>,
        min: usize,
        max: Option<usize>,
    },
}

#[derive(Debug, Clone)]
struct CharClass {
    negated: bool,
    items: Vec<ClassItem>,
}

#[derive(Debug, Clone)]
enum ClassItem {
    Char(char),
    Range(char, char),
    PosixClass(PosixClass),
}

#[derive(Debug, Clone)]
enum PosixClass {
    Alnum,
    Alpha,
    Blank,
    Cntrl,
    Digit,
    Graph,
    Lower,
    Print,
    Punct,
    Space,
    Upper,
    Xdigit,
}

pub fn compile(pattern: &str) -> Result<PosixRegex, String> {
    let mut parser = Parser::new(pattern);
    let ast = parser.parse()?;
    Ok(PosixRegex { ast })
}

impl PosixRegex {
    pub fn is_match(&self, text: &str) -> bool {
        self.find(text).is_some()
    }

    pub fn find(&self, text: &str) -> Option<MatchSpan> {
        self.find_from(text, 0)
    }

    pub fn find_from(&self, text: &str, start: usize) -> Option<MatchSpan> {
        let input = Input::new(text);
        let mut cache = HashMap::new();
        for start_pos in start..=input.len_chars() {
            let ends = self.ast.match_positions(&input, start_pos, &mut cache);
            if let Some(end_pos) = ends.into_iter().max() {
                return Some(MatchSpan {
                    start: input.byte_offset(start_pos),
                    end: input.byte_offset(end_pos),
                });
            }
        }
        None
    }

    pub fn split<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let mut parts = Vec::new();
        let mut last_end = 0usize;
        let mut search_from = 0usize;

        while let Some(matched) = self.find_from(text, search_from) {
            parts.push(&text[last_end..matched.start]);

            if matched.start == matched.end {
                if let Some(next) = text[matched.end..].chars().next() {
                    let next_end = matched.end + next.len_utf8();
                    last_end = next_end;
                    search_from = count_chars(&text[..next_end]);
                } else {
                    last_end = matched.end;
                    break;
                }
            } else {
                last_end = matched.end;
                search_from = count_chars(&text[..matched.end]);
            }
        }

        parts.push(&text[last_end..]);
        parts
    }
}

#[derive(Debug)]
struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(pattern: &str) -> Self {
        Self {
            chars: pattern.chars().collect(),
            pos: 0,
        }
    }

    fn parse(&mut self) -> Result<Expr, String> {
        let expr = self.parse_alternate()?;
        if self.pos != self.chars.len() {
            return Err(format!(
                "unexpected regex character '{}'",
                self.chars[self.pos]
            ));
        }
        Ok(expr)
    }

    fn parse_alternate(&mut self) -> Result<Expr, String> {
        let mut branches = vec![self.parse_concat()?];
        while self.peek() == Some('|') {
            self.pos += 1;
            branches.push(self.parse_concat()?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Expr::Alternate(branches)
        })
    }

    fn parse_concat(&mut self) -> Result<Expr, String> {
        let mut parts = Vec::new();
        let mut at_branch_start = true;
        while let Some(ch) = self.peek() {
            if ch == ')' || ch == '|' {
                break;
            }
            parts.push(self.parse_repeat(at_branch_start)?);
            at_branch_start = false;
        }

        Ok(match parts.len() {
            0 => Expr::Empty,
            1 => parts.remove(0),
            _ => Expr::Concat(parts),
        })
    }

    fn parse_repeat(&mut self, at_branch_start: bool) -> Result<Expr, String> {
        let mut expr = self.parse_atom(at_branch_start)?;
        loop {
            expr = match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    Expr::Repeat {
                        expr: Box::new(expr),
                        min: 0,
                        max: None,
                    }
                }
                Some('+') => {
                    self.pos += 1;
                    Expr::Repeat {
                        expr: Box::new(expr),
                        min: 1,
                        max: None,
                    }
                }
                Some('?') => {
                    self.pos += 1;
                    Expr::Repeat {
                        expr: Box::new(expr),
                        min: 0,
                        max: Some(1),
                    }
                }
                Some('{') => {
                    let (min, max) = self.parse_counted_repeat()?;
                    Expr::Repeat {
                        expr: Box::new(expr),
                        min,
                        max,
                    }
                }
                _ => break,
            };
        }
        Ok(expr)
    }

    fn parse_counted_repeat(&mut self) -> Result<(usize, Option<usize>), String> {
        self.expect('{')?;
        let min = self.parse_usize()?;
        let max = if self.peek() == Some(',') {
            self.pos += 1;
            if self.peek() == Some('}') {
                None
            } else {
                Some(self.parse_usize()?)
            }
        } else {
            Some(min)
        };
        self.expect('}')?;
        if let Some(max) = max {
            if max < min {
                return Err(format!(
                    "invalid repeat range: lower bound {} exceeds upper bound {}",
                    min, max
                ));
            }
        }
        Ok((min, max))
    }

    fn parse_atom(&mut self, at_branch_start: bool) -> Result<Expr, String> {
        match self
            .next()
            .ok_or_else(|| "unexpected end of regex".to_string())?
        {
            '(' => {
                let expr = self.parse_alternate()?;
                self.expect(')')?;
                Ok(expr)
            }
            '.' => Ok(Expr::Dot),
            '^' if at_branch_start => Ok(Expr::AnchorStart),
            '^' => Ok(Expr::Literal('^')),
            '$' if self.is_branch_end() => Ok(Expr::AnchorEnd),
            '$' => Ok(Expr::Literal('$')),
            '[' => self.parse_class(),
            '\\' => self.parse_escape(),
            ch => Ok(Expr::Literal(ch)),
        }
    }

    fn parse_escape(&mut self) -> Result<Expr, String> {
        let ch = self
            .next()
            .ok_or_else(|| "unterminated escape in regex".to_string())?;
        match ch {
            'd' => Ok(Expr::Class(CharClass::single_posix(PosixClass::Digit))),
            'D' => Ok(Expr::Class(CharClass::negated_posix(PosixClass::Digit))),
            's' => Ok(Expr::Class(CharClass::single_posix(PosixClass::Space))),
            'S' => Ok(Expr::Class(CharClass::negated_posix(PosixClass::Space))),
            'w' => Ok(Expr::Class(CharClass {
                negated: false,
                items: vec![
                    ClassItem::PosixClass(PosixClass::Alnum),
                    ClassItem::Char('_'),
                ],
            })),
            'W' => Ok(Expr::Class(CharClass {
                negated: true,
                items: vec![
                    ClassItem::PosixClass(PosixClass::Alnum),
                    ClassItem::Char('_'),
                ],
            })),
            '0'..='9' => Err(format!(
                "backreferences are not supported in POSIX ERE: \\{}",
                ch
            )),
            'b' | 'B' | 'A' | 'z' => {
                Err(format!("unsupported regex escape for POSIX ERE: \\{}", ch))
            }
            other => Ok(Expr::Literal(other)),
        }
    }

    fn parse_class(&mut self) -> Result<Expr, String> {
        let mut negated = false;
        if self.peek() == Some('^') {
            self.pos += 1;
            negated = true;
        }

        let mut items = Vec::new();
        let mut first = true;
        if matches!(self.peek(), Some(']' | '-')) {
            items.push(ClassItem::Char(self.next().unwrap()));
            first = false;
        }
        while let Some(ch) = self.peek() {
            if ch == ']' && !first {
                self.pos += 1;
                return Ok(Expr::Class(CharClass { negated, items }));
            }
            first = false;

            if ch == '[' && self.peek_n(1) == Some(':') {
                self.pos += 2;
                let class_name = self.read_until(":]")?;
                items.push(ClassItem::PosixClass(parse_posix_class(&class_name)?));
                continue;
            }
            if ch == '[' && self.peek_n(1) == Some('.') {
                return Err(
                    "collating symbols are not supported; regexes use C-locale semantics"
                        .to_string(),
                );
            }
            if ch == '[' && self.peek_n(1) == Some('=') {
                return Err(
                    "equivalence classes are not supported; regexes use C-locale semantics"
                        .to_string(),
                );
            }

            let start = self.parse_class_char()?;
            if self.peek() == Some('-') && self.peek_n(1) != Some(']') {
                self.pos += 1;
                let end = self.parse_class_char()?;
                if start > end {
                    return Err(format!("invalid character class range: {}-{}", start, end));
                }
                items.push(ClassItem::Range(start, end));
            } else {
                items.push(ClassItem::Char(start));
            }
        }

        Err("unterminated character class".to_string())
    }

    fn parse_class_char(&mut self) -> Result<char, String> {
        let ch = self
            .next()
            .ok_or_else(|| "unterminated character class".to_string())?;
        if ch == '\\' {
            self.next()
                .ok_or_else(|| "unterminated escape in character class".to_string())
        } else {
            Ok(ch)
        }
    }

    fn parse_usize(&mut self) -> Result<usize, String> {
        let start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err("expected repeat count".to_string());
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .parse::<usize>()
            .map_err(|_| "invalid repeat count".to_string())
    }

    fn read_until(&mut self, terminator: &str) -> Result<String, String> {
        let terminator_chars: Vec<char> = terminator.chars().collect();
        let start = self.pos;
        while self.pos + terminator_chars.len() <= self.chars.len() {
            if self.chars[self.pos..self.pos + terminator_chars.len()] == terminator_chars[..] {
                let value = self.chars[start..self.pos].iter().collect::<String>();
                self.pos += terminator_chars.len();
                return Ok(value);
            }
            self.pos += 1;
        }
        Err("unterminated POSIX character class".to_string())
    }

    fn expect(&mut self, ch: char) -> Result<(), String> {
        match self.next() {
            Some(actual) if actual == ch => Ok(()),
            Some(actual) => Err(format!("expected '{}', got '{}'", ch, actual)),
            None => Err(format!("expected '{}', got end of regex", ch)),
        }
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn is_branch_end(&self) -> bool {
        matches!(self.peek(), None | Some(')') | Some('|'))
    }
}

impl Expr {
    fn match_positions(
        &self,
        input: &Input<'_>,
        pos: usize,
        cache: &mut HashMap<(usize, usize), BTreeSet<usize>>,
    ) -> BTreeSet<usize> {
        let key = (self as *const Expr as usize, pos);
        if let Some(cached) = cache.get(&key) {
            return cached.clone();
        }

        let result = match self {
            Expr::Empty => BTreeSet::from([pos]),
            Expr::Literal(ch) => input
                .char_at(pos)
                .filter(|actual| actual == ch)
                .map(|_| BTreeSet::from([pos + 1]))
                .unwrap_or_default(),
            Expr::Dot => input
                .char_at(pos)
                .filter(|ch| *ch != '\0')
                .map(|_| BTreeSet::from([pos + 1]))
                .unwrap_or_default(),
            Expr::Class(class) => input
                .char_at(pos)
                .filter(|ch| class.matches(*ch))
                .map(|_| BTreeSet::from([pos + 1]))
                .unwrap_or_default(),
            Expr::AnchorStart => {
                if pos == 0 {
                    BTreeSet::from([pos])
                } else {
                    BTreeSet::new()
                }
            }
            Expr::AnchorEnd => {
                if pos == input.len_chars() {
                    BTreeSet::from([pos])
                } else {
                    BTreeSet::new()
                }
            }
            Expr::Concat(parts) => {
                let mut current = BTreeSet::from([pos]);
                for part in parts {
                    let mut next_positions = BTreeSet::new();
                    for current_pos in current {
                        next_positions.extend(part.match_positions(input, current_pos, cache));
                    }
                    if next_positions.is_empty() {
                        cache.insert(key, next_positions.clone());
                        return next_positions;
                    }
                    current = next_positions;
                }
                current
            }
            Expr::Alternate(branches) => {
                let mut out = BTreeSet::new();
                for branch in branches {
                    out.extend(branch.match_positions(input, pos, cache));
                }
                out
            }
            Expr::Repeat { expr, min, max } => repeat_match(expr, input, pos, *min, *max, cache),
        };

        cache.insert(key, result.clone());
        result
    }
}

fn repeat_match(
    expr: &Expr,
    input: &Input<'_>,
    pos: usize,
    min: usize,
    max: Option<usize>,
    cache: &mut HashMap<(usize, usize), BTreeSet<usize>>,
) -> BTreeSet<usize> {
    let mut results = BTreeSet::new();
    let mut current = BTreeSet::from([pos]);
    let repetition_limit = max.unwrap_or(input.len_chars().saturating_sub(pos));

    if min == 0 {
        results.insert(pos);
    }

    for count in 1..=repetition_limit {
        let mut next = BTreeSet::new();
        for current_pos in &current {
            for end_pos in expr.match_positions(input, *current_pos, cache) {
                if end_pos != *current_pos {
                    next.insert(end_pos);
                }
            }
        }

        if next.is_empty() {
            break;
        }

        if count >= min {
            results.extend(next.iter().copied());
        }

        current = next;
    }

    results
}

impl CharClass {
    fn single_posix(class: PosixClass) -> Self {
        Self {
            negated: false,
            items: vec![ClassItem::PosixClass(class)],
        }
    }

    fn negated_posix(class: PosixClass) -> Self {
        Self {
            negated: true,
            items: vec![ClassItem::PosixClass(class)],
        }
    }

    fn matches(&self, ch: char) -> bool {
        let matched = self.items.iter().any(|item| match item {
            ClassItem::Char(single) => *single == ch,
            ClassItem::Range(start, end) => *start <= ch && ch <= *end,
            ClassItem::PosixClass(class) => class.matches(ch),
        });
        if self.negated {
            !matched
        } else {
            matched
        }
    }
}

impl PosixClass {
    fn matches(&self, ch: char) -> bool {
        match self {
            PosixClass::Alnum => ch.is_ascii_alphanumeric(),
            PosixClass::Alpha => ch.is_ascii_alphabetic(),
            PosixClass::Blank => matches!(ch, ' ' | '\t'),
            PosixClass::Cntrl => ch.is_ascii_control(),
            PosixClass::Digit => ch.is_ascii_digit(),
            PosixClass::Graph => ch.is_ascii_graphic(),
            PosixClass::Lower => ch.is_ascii_lowercase(),
            PosixClass::Print => ch.is_ascii() && !ch.is_ascii_control(),
            PosixClass::Punct => ch.is_ascii_punctuation(),
            PosixClass::Space => ch.is_ascii_whitespace(),
            PosixClass::Upper => ch.is_ascii_uppercase(),
            PosixClass::Xdigit => ch.is_ascii_hexdigit(),
        }
    }
}

fn parse_posix_class(name: &str) -> Result<PosixClass, String> {
    match name {
        "alnum" => Ok(PosixClass::Alnum),
        "alpha" => Ok(PosixClass::Alpha),
        "blank" => Ok(PosixClass::Blank),
        "cntrl" => Ok(PosixClass::Cntrl),
        "digit" => Ok(PosixClass::Digit),
        "graph" => Ok(PosixClass::Graph),
        "lower" => Ok(PosixClass::Lower),
        "print" => Ok(PosixClass::Print),
        "punct" => Ok(PosixClass::Punct),
        "space" => Ok(PosixClass::Space),
        "upper" => Ok(PosixClass::Upper),
        "xdigit" => Ok(PosixClass::Xdigit),
        _ => Err(format!("unsupported POSIX character class: {}", name)),
    }
}

struct Input<'a> {
    chars: Vec<char>,
    byte_offsets: Vec<usize>,
    _text: &'a str,
}

impl<'a> Input<'a> {
    fn new(text: &'a str) -> Self {
        let mut chars = Vec::new();
        let mut byte_offsets = Vec::new();
        for (offset, ch) in text.char_indices() {
            chars.push(ch);
            byte_offsets.push(offset);
        }
        byte_offsets.push(text.len());
        Self {
            chars,
            byte_offsets,
            _text: text,
        }
    }

    fn len_chars(&self) -> usize {
        self.chars.len()
    }

    fn char_at(&self, pos: usize) -> Option<char> {
        self.chars.get(pos).copied()
    }

    fn byte_offset(&self, pos: usize) -> usize {
        self.byte_offsets[pos]
    }
}

fn count_chars(text: &str) -> usize {
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_alternation_prefers_longest_match() {
        let re = compile("a|ab").unwrap();
        assert_eq!(re.find("ab"), Some(MatchSpan { start: 0, end: 2 }));
    }

    #[test]
    fn nested_alternation_prefers_longest_match() {
        let re = compile("(a|ab)c").unwrap();
        assert_eq!(re.find("abc"), Some(MatchSpan { start: 0, end: 3 }));
    }

    #[test]
    fn shorthand_classes_are_translated() {
        let re = compile(r"\d+").unwrap();
        assert_eq!(re.find("abc123"), Some(MatchSpan { start: 3, end: 6 }));
    }

    #[test]
    fn posix_character_class_uses_c_locale_ascii_rules() {
        let re = compile("[[:alpha:]]+").unwrap();
        assert_eq!(re.find("123ABC"), Some(MatchSpan { start: 3, end: 6 }));
        assert_eq!(re.find("é"), None);
    }

    #[test]
    fn anchors_are_context_sensitive() {
        let re = compile("a^b").unwrap();
        assert_eq!(re.find("a^b"), Some(MatchSpan { start: 0, end: 3 }));

        let re = compile("ab$").unwrap();
        assert_eq!(re.find("xxab"), Some(MatchSpan { start: 2, end: 4 }));
    }

    #[test]
    fn leading_bracket_and_dash_are_literals_in_class() {
        let re = compile("[]-]+").unwrap();
        assert_eq!(re.find("abc-]"), Some(MatchSpan { start: 3, end: 5 }));
    }

    #[test]
    fn collating_symbols_are_rejected() {
        let err = compile("[[.ch.]]").unwrap_err();
        assert!(err.contains("collating symbols"));
    }

    #[test]
    fn equivalence_classes_are_rejected() {
        let err = compile("[[=a=]]").unwrap_err();
        assert!(err.contains("equivalence classes"));
    }

    #[test]
    fn invalid_repeat_range_is_rejected() {
        let err = compile("a{3,2}").unwrap_err();
        assert!(err.contains("invalid repeat range"));
    }

    #[test]
    fn invalid_character_class_range_is_rejected() {
        let err = compile("[z-a]").unwrap_err();
        assert!(err.contains("invalid character class range"));
    }
}
