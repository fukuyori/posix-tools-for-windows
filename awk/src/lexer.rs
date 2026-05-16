/// AWK Lexer - Tokenizes AWK source code
use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Number(f64),
    String(String),
    Regex(String),

    // Identifiers and keywords
    Identifier(String),
    Begin,
    End,
    If,
    Else,
    While,
    For,
    In,
    Do,
    Break,
    Continue,
    Next,
    NextFile,
    Exit,
    Return,
    Function,
    Delete,
    Print,
    Printf,
    Getline,

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    PlusPlus,
    MinusMinus,

    // Assignment
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    CaretAssign,

    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Match,    // ~
    NotMatch, // !~

    // Logical
    And, // &&
    Or,  // ||
    Not, // !

    // Other operators
    Question,
    Colon,
    Dollar, // Field access $
    Append, // >>
    Pipe,   // |

    // Delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Newline,

    // Special
    Eof,
}

#[derive(Debug, Clone)]
pub struct LexerError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for LexerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lexer error at {}:{}: {}",
            self.line, self.column, self.message
        )
    }
}

pub struct Lexer<'a> {
    input: Peekable<Chars<'a>>,
    current_char: Option<char>,
    line: usize,
    column: usize,
    at_line_start: bool,
    last_token: Option<Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        let mut lexer = Lexer {
            input: input.chars().peekable(),
            current_char: None,
            line: 1,
            column: 0,
            at_line_start: true,
            last_token: None,
        };
        lexer.advance();
        lexer
    }

    fn advance(&mut self) {
        self.current_char = self.input.next();
        if let Some(c) = self.current_char {
            if c == '\n' {
                self.line += 1;
                self.column = 0;
            } else {
                self.column += 1;
            }
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.input.peek().copied()
    }

    fn error(&self, message: &str) -> LexerError {
        LexerError {
            message: message.to_string(),
            line: self.line,
            column: self.column,
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.current_char {
            if c == ' ' || c == '\t' || c == '\r' {
                self.advance();
            } else if c == '\\' {
                // Line continuation
                if self.peek() == Some('\n') {
                    self.advance(); // skip \
                    self.advance(); // skip \n
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(c) = self.current_char {
            if c == '\n' {
                break;
            }
            self.advance();
        }
    }

    fn read_number(&mut self) -> Result<Token, LexerError> {
        let mut num_str = String::new();

        // Integer part
        while let Some(c) = self.current_char {
            if c.is_ascii_digit() {
                num_str.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Decimal part
        if self.current_char == Some('.') {
            if let Some(next) = self.peek() {
                if next.is_ascii_digit() {
                    num_str.push('.');
                    self.advance();
                    while let Some(c) = self.current_char {
                        if c.is_ascii_digit() {
                            num_str.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
            }
        }

        // Exponent part
        if let Some(c) = self.current_char {
            if c == 'e' || c == 'E' {
                num_str.push(c);
                self.advance();
                if let Some(sign) = self.current_char {
                    if sign == '+' || sign == '-' {
                        num_str.push(sign);
                        self.advance();
                    }
                }
                while let Some(c) = self.current_char {
                    if c.is_ascii_digit() {
                        num_str.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        num_str
            .parse::<f64>()
            .map(Token::Number)
            .map_err(|_| self.error("Invalid number"))
    }

    fn read_string(&mut self) -> Result<Token, LexerError> {
        self.advance(); // skip opening quote
        let mut s = String::new();

        while let Some(c) = self.current_char {
            match c {
                '"' => {
                    self.advance();
                    return Ok(Token::String(s));
                }
                '\\' => {
                    self.advance();
                    match self.current_char {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('r') => s.push('\r'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some('/') => s.push('/'),
                        Some('b') => s.push('\x08'),
                        Some('f') => s.push('\x0c'),
                        Some(c) => s.push(c),
                        None => return Err(self.error("Unexpected end of input in string")),
                    }
                    self.advance();
                }
                '\n' => return Err(self.error("Unterminated string")),
                _ => {
                    s.push(c);
                    self.advance();
                }
            }
        }

        Err(self.error("Unterminated string"))
    }

    fn read_regex(&mut self) -> Result<Token, LexerError> {
        self.advance(); // skip opening /
        let mut pattern = String::new();

        while let Some(c) = self.current_char {
            match c {
                '/' => {
                    self.advance();
                    return Ok(Token::Regex(pattern));
                }
                '\\' => {
                    pattern.push(c);
                    self.advance();
                    if let Some(c) = self.current_char {
                        pattern.push(c);
                        self.advance();
                    }
                }
                '\n' => return Err(self.error("Unterminated regex")),
                _ => {
                    pattern.push(c);
                    self.advance();
                }
            }
        }

        Err(self.error("Unterminated regex"))
    }

    fn read_identifier(&mut self) -> Token {
        let mut ident = String::new();

        while let Some(c) = self.current_char {
            if c.is_ascii_alphanumeric() || c == '_' {
                ident.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Check for keywords
        match ident.as_str() {
            "BEGIN" => Token::Begin,
            "END" => Token::End,
            "if" => Token::If,
            "else" => Token::Else,
            "while" => Token::While,
            "for" => Token::For,
            "in" => Token::In,
            "do" => Token::Do,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "next" => Token::Next,
            "nextfile" => Token::NextFile,
            "exit" => Token::Exit,
            "return" => Token::Return,
            "function" => Token::Function,
            "delete" => Token::Delete,
            "print" => Token::Print,
            "printf" => Token::Printf,
            "getline" => Token::Getline,
            _ => Token::Identifier(ident),
        }
    }

    /// Check if a '/' should be interpreted as regex or division
    fn expecting_regex(&self) -> bool {
        match &self.last_token {
            None => true,
            Some(tok) => matches!(
                tok,
                Token::LParen
                    | Token::LBrace
                    | Token::RBrace
                    | Token::LBracket
                    | Token::Comma
                    | Token::Semicolon
                    | Token::Newline
                    | Token::And
                    | Token::Or
                    | Token::Not
                    | Token::Match
                    | Token::NotMatch
                    | Token::Question
                    | Token::Colon
                    | Token::Assign
                    | Token::PlusAssign
                    | Token::MinusAssign
                    | Token::StarAssign
                    | Token::SlashAssign
                    | Token::PercentAssign
                    | Token::CaretAssign
                    | Token::Eq
                    | Token::Ne
                    | Token::Lt
                    | Token::Le
                    | Token::Gt
                    | Token::Ge
                    | Token::Print
                    | Token::Printf
                    | Token::If
                    | Token::While
                    | Token::For
                    | Token::Do
                    | Token::Return
            ),
        }
    }

    pub fn next_token(&mut self) -> Result<Token, LexerError> {
        self.skip_whitespace();

        let token = match self.current_char {
            None => Ok(Token::Eof),
            Some(c) => match c {
                '#' => {
                    self.skip_comment();
                    self.next_token()
                }
                '\n' => {
                    self.advance();
                    self.at_line_start = true;
                    Ok(Token::Newline)
                }
                '"' => self.read_string(),
                '/' => {
                    if self.expecting_regex() {
                        self.read_regex()
                    } else {
                        self.advance();
                        if self.current_char == Some('=') {
                            self.advance();
                            Ok(Token::SlashAssign)
                        } else {
                            Ok(Token::Slash)
                        }
                    }
                }
                '0'..='9' => self.read_number(),
                '.' => {
                    if let Some(next) = self.peek() {
                        if next.is_ascii_digit() {
                            self.read_number()
                        } else {
                            Err(self.error("Unexpected character '.'"))
                        }
                    } else {
                        Err(self.error("Unexpected character '.'"))
                    }
                }
                'a'..='z' | 'A'..='Z' | '_' => Ok(self.read_identifier()),
                '+' => {
                    self.advance();
                    match self.current_char {
                        Some('+') => {
                            self.advance();
                            Ok(Token::PlusPlus)
                        }
                        Some('=') => {
                            self.advance();
                            Ok(Token::PlusAssign)
                        }
                        _ => Ok(Token::Plus),
                    }
                }
                '-' => {
                    self.advance();
                    match self.current_char {
                        Some('-') => {
                            self.advance();
                            Ok(Token::MinusMinus)
                        }
                        Some('=') => {
                            self.advance();
                            Ok(Token::MinusAssign)
                        }
                        _ => Ok(Token::Minus),
                    }
                }
                '*' => {
                    self.advance();
                    if self.current_char == Some('=') {
                        self.advance();
                        Ok(Token::StarAssign)
                    } else {
                        Ok(Token::Star)
                    }
                }
                '%' => {
                    self.advance();
                    if self.current_char == Some('=') {
                        self.advance();
                        Ok(Token::PercentAssign)
                    } else {
                        Ok(Token::Percent)
                    }
                }
                '^' => {
                    self.advance();
                    if self.current_char == Some('=') {
                        self.advance();
                        Ok(Token::CaretAssign)
                    } else {
                        Ok(Token::Caret)
                    }
                }
                '=' => {
                    self.advance();
                    if self.current_char == Some('=') {
                        self.advance();
                        Ok(Token::Eq)
                    } else {
                        Ok(Token::Assign)
                    }
                }
                '!' => {
                    self.advance();
                    match self.current_char {
                        Some('=') => {
                            self.advance();
                            Ok(Token::Ne)
                        }
                        Some('~') => {
                            self.advance();
                            Ok(Token::NotMatch)
                        }
                        _ => Ok(Token::Not),
                    }
                }
                '<' => {
                    self.advance();
                    if self.current_char == Some('=') {
                        self.advance();
                        Ok(Token::Le)
                    } else {
                        Ok(Token::Lt)
                    }
                }
                '>' => {
                    self.advance();
                    match self.current_char {
                        Some('=') => {
                            self.advance();
                            Ok(Token::Ge)
                        }
                        Some('>') => {
                            self.advance();
                            Ok(Token::Append)
                        }
                        _ => Ok(Token::Gt),
                    }
                }
                '~' => {
                    self.advance();
                    Ok(Token::Match)
                }
                '&' => {
                    self.advance();
                    if self.current_char == Some('&') {
                        self.advance();
                        Ok(Token::And)
                    } else {
                        Err(self.error("Expected '&&'"))
                    }
                }
                '|' => {
                    self.advance();
                    if self.current_char == Some('|') {
                        self.advance();
                        Ok(Token::Or)
                    } else {
                        Ok(Token::Pipe)
                    }
                }
                '?' => {
                    self.advance();
                    Ok(Token::Question)
                }
                ':' => {
                    self.advance();
                    Ok(Token::Colon)
                }
                '$' => {
                    self.advance();
                    Ok(Token::Dollar)
                }
                '(' => {
                    self.advance();
                    Ok(Token::LParen)
                }
                ')' => {
                    self.advance();
                    Ok(Token::RParen)
                }
                '{' => {
                    self.advance();
                    Ok(Token::LBrace)
                }
                '}' => {
                    self.advance();
                    Ok(Token::RBrace)
                }
                '[' => {
                    self.advance();
                    Ok(Token::LBracket)
                }
                ']' => {
                    self.advance();
                    Ok(Token::RBracket)
                }
                ',' => {
                    self.advance();
                    Ok(Token::Comma)
                }
                ';' => {
                    self.advance();
                    Ok(Token::Semicolon)
                }
                _ => Err(self.error(&format!("Unexpected character '{}'", c))),
            },
        };

        if let Ok(ref tok) = token {
            if !matches!(tok, Token::Newline) {
                self.at_line_start = false;
            }
            self.last_token = Some(tok.clone());
        }

        token
    }

    #[allow(dead_code)]
    pub fn tokenize(&mut self) -> Result<Vec<Token>, LexerError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            if matches!(token, Token::Eof) {
                tokens.push(token);
                break;
            }
            tokens.push(token);
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let mut lexer = Lexer::new("{ print $1 }");
        assert!(matches!(lexer.next_token(), Ok(Token::LBrace)));
        assert!(matches!(lexer.next_token(), Ok(Token::Print)));
        assert!(matches!(lexer.next_token(), Ok(Token::Dollar)));
        assert!(matches!(lexer.next_token(), Ok(Token::Number(n)) if n == 1.0));
        assert!(matches!(lexer.next_token(), Ok(Token::RBrace)));
    }

    #[test]
    fn test_string() {
        let mut lexer = Lexer::new("\"hello\\nworld\"");
        assert!(matches!(lexer.next_token(), Ok(Token::String(s)) if s == "hello\nworld"));
    }

    #[test]
    fn test_regex() {
        let mut lexer = Lexer::new("/^test$/");
        assert!(matches!(lexer.next_token(), Ok(Token::Regex(r)) if r == "^test$"));
    }

    #[test]
    fn regex_after_action_block() {
        // `{action} /regex/` — `/` after `}` must lex as a regex literal,
        // not as division. This is the start of the next rule's pattern.
        let mut lexer = Lexer::new("{next} /^#/");
        let mut last = None;
        loop {
            let t = lexer.next_token().unwrap();
            if matches!(t, Token::Eof) {
                break;
            }
            last = Some(t);
        }
        assert!(matches!(last, Some(Token::Regex(p)) if p == "^#"));
    }

    #[test]
    fn shebang_regex_in_compound_rule() {
        // Full reproduction of the reported bug:
        //   NR==1 && /^#!/ {next} /^#/ {print}
        // Both `/^#!/` and `/^#/` must be lexed as Regex tokens.
        let mut lexer = Lexer::new("NR==1 && /^#!/ {next} /^#/ {print}");
        let mut regexes = Vec::new();
        loop {
            let t = lexer.next_token().unwrap();
            if matches!(t, Token::Eof) {
                break;
            }
            if let Token::Regex(p) = t {
                regexes.push(p);
            }
        }
        assert_eq!(regexes, vec!["^#!".to_string(), "^#".to_string()]);
    }
}
