/// AWK Parser - Parses tokens into an AST
use crate::ast::*;
use crate::lexer::{Lexer, LexerError, Token};

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Parse error: {}", self.message)
    }
}

impl From<LexerError> for ParseError {
    fn from(e: LexerError) -> Self {
        ParseError {
            message: e.to_string(),
        }
    }
}

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    peeked: Option<Token>,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a str) -> Result<Self, ParseError> {
        let mut lexer = Lexer::new(input);
        let current = lexer.next_token()?;
        Ok(Parser {
            lexer,
            current,
            peeked: None,
        })
    }

    fn error(&self, msg: &str) -> ParseError {
        ParseError {
            message: msg.to_string(),
        }
    }

    fn advance(&mut self) -> Result<Token, ParseError> {
        let old = std::mem::replace(
            &mut self.current,
            if let Some(tok) = self.peeked.take() {
                tok
            } else {
                self.lexer.next_token()?
            },
        );
        Ok(old)
    }

    #[allow(dead_code)]
    fn peek(&mut self) -> Result<&Token, ParseError> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token()?);
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    fn expect(&mut self, expected: Token) -> Result<(), ParseError> {
        if std::mem::discriminant(&self.current) == std::mem::discriminant(&expected) {
            self.advance()?;
            Ok(())
        } else {
            Err(self.error(&format!("Expected {:?}, got {:?}", expected, self.current)))
        }
    }

    fn skip_newlines(&mut self) -> Result<(), ParseError> {
        while matches!(self.current, Token::Newline) {
            self.advance()?;
        }
        Ok(())
    }

    fn skip_terminators(&mut self) -> Result<(), ParseError> {
        while matches!(self.current, Token::Newline | Token::Semicolon) {
            self.advance()?;
        }
        Ok(())
    }

    /// Parse the complete program
    pub fn parse(&mut self) -> Result<Program, ParseError> {
        let mut program = Program::new();

        self.skip_newlines()?;

        while !matches!(self.current, Token::Eof) {
            if matches!(self.current, Token::Function) {
                program.functions.push(self.parse_function()?);
            } else {
                program.rules.push(self.parse_rule()?);
            }
            self.skip_terminators()?;
        }

        Ok(program)
    }

    /// Parse a function definition
    fn parse_function(&mut self) -> Result<Function, ParseError> {
        self.expect(Token::Function)?;

        let name = match &self.current {
            Token::Identifier(n) => n.clone(),
            _ => return Err(self.error("Expected function name")),
        };
        self.advance()?;

        self.expect(Token::LParen)?;

        let mut params = Vec::new();
        while !matches!(self.current, Token::RParen) {
            match &self.current {
                Token::Identifier(p) => params.push(p.clone()),
                _ => return Err(self.error("Expected parameter name")),
            }
            self.advance()?;
            if matches!(self.current, Token::Comma) {
                self.advance()?;
            }
        }
        self.expect(Token::RParen)?;
        self.skip_newlines()?;

        let body = self.parse_block()?;

        Ok(Function { name, params, body })
    }

    /// Parse a rule (pattern-action pair)
    fn parse_rule(&mut self) -> Result<Rule, ParseError> {
        let pattern = self.parse_pattern()?;
        self.skip_newlines()?;

        let action = if matches!(self.current, Token::LBrace) {
            self.parse_block()?
        } else if pattern.is_some() {
            // Pattern without action: default to { print }
            vec![Stmt::Print {
                args: vec![],
                output: None,
            }]
        } else {
            return Err(self.error("Expected pattern or action"));
        };

        Ok(Rule { pattern, action })
    }

    /// Parse a pattern
    fn parse_pattern(&mut self) -> Result<Option<Pattern>, ParseError> {
        match &self.current {
            Token::Begin => {
                self.advance()?;
                Ok(Some(Pattern::Begin))
            }
            Token::End => {
                self.advance()?;
                Ok(Some(Pattern::End))
            }
            Token::LBrace => Ok(None),
            Token::Regex(r) => {
                let regex = r.clone();
                self.advance()?;
                let pat = Pattern::Expr(Expr::Regex(regex));
                self.check_range_pattern(pat)
            }
            Token::Eof => Ok(None),
            _ => {
                let expr = self.parse_expr()?;
                let pat = Pattern::Expr(expr);
                self.check_range_pattern(pat)
            }
        }
    }

    /// Check for range pattern (pat1, pat2)
    fn check_range_pattern(&mut self, start: Pattern) -> Result<Option<Pattern>, ParseError> {
        if matches!(self.current, Token::Comma) {
            self.advance()?;
            self.skip_newlines()?;
            let end = match self.parse_pattern()? {
                Some(p) => p,
                None => return Err(self.error("Expected end pattern in range")),
            };
            Ok(Some(Pattern::Range {
                start: Box::new(start),
                end: Box::new(end),
            }))
        } else {
            Ok(Some(start))
        }
    }

    /// Parse a block { ... }
    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.expect(Token::LBrace)?;
        self.skip_newlines()?;

        let mut stmts = Vec::new();
        while !matches!(self.current, Token::RBrace | Token::Eof) {
            stmts.push(self.parse_stmt()?);
            self.skip_terminators()?;
        }

        self.expect(Token::RBrace)?;
        Ok(stmts)
    }

    /// Parse a statement
    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match &self.current {
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::Do => self.parse_do_while(),
            Token::For => self.parse_for(),
            Token::LBrace => Ok(Stmt::Block(self.parse_block()?)),
            Token::Break => {
                self.advance()?;
                Ok(Stmt::Break)
            }
            Token::Continue => {
                self.advance()?;
                Ok(Stmt::Continue)
            }
            Token::Next => {
                self.advance()?;
                Ok(Stmt::Next)
            }
            Token::NextFile => {
                self.advance()?;
                Ok(Stmt::NextFile)
            }
            Token::Exit => self.parse_exit(),
            Token::Return => self.parse_return(),
            Token::Delete => self.parse_delete(),
            Token::Print => self.parse_print(),
            Token::Printf => self.parse_printf(),
            Token::Semicolon | Token::Newline => Ok(Stmt::Empty),
            _ => {
                let expr = self.parse_expr()?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::If)?;
        self.expect(Token::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(Token::RParen)?;
        self.skip_newlines()?;

        let then_branch = Box::new(self.parse_stmt()?);
        self.skip_newlines()?;

        let else_branch = if matches!(self.current, Token::Else) {
            self.advance()?;
            self.skip_newlines()?;
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };

        Ok(Stmt::If {
            cond,
            then_branch,
            else_branch,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::While)?;
        self.expect(Token::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(Token::RParen)?;
        self.skip_newlines()?;
        let body = Box::new(self.parse_stmt()?);

        Ok(Stmt::While { cond, body })
    }

    fn parse_do_while(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::Do)?;
        self.skip_newlines()?;
        let body = Box::new(self.parse_stmt()?);
        self.skip_newlines()?;
        self.expect(Token::While)?;
        self.expect(Token::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(Token::RParen)?;

        Ok(Stmt::DoWhile { body, cond })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::For)?;
        self.expect(Token::LParen)?;

        // Check for for-in loop: for (var in array)
        if let Token::Identifier(var) = &self.current {
            let var_name = var.clone();
            self.advance()?;

            if matches!(self.current, Token::In) {
                self.advance()?;
                let array = match &self.current {
                    Token::Identifier(a) => a.clone(),
                    _ => return Err(self.error("Expected array name")),
                };
                self.advance()?;
                self.expect(Token::RParen)?;
                self.skip_newlines()?;
                let body = Box::new(self.parse_stmt()?);

                return Ok(Stmt::ForIn {
                    var: var_name,
                    array,
                    body,
                });
            }

            // Not for-in, parse as regular for with the identifier as init
            let init = Some(self.parse_assignment_from_var(var_name)?);
            self.expect(Token::Semicolon)?;

            let cond = if matches!(self.current, Token::Semicolon) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            self.expect(Token::Semicolon)?;

            let update = if matches!(self.current, Token::RParen) {
                None
            } else {
                Some(self.parse_expr()?)
            };
            self.expect(Token::RParen)?;
            self.skip_newlines()?;
            let body = Box::new(self.parse_stmt()?);

            return Ok(Stmt::For {
                init,
                cond,
                update,
                body,
            });
        }

        // Regular for loop
        let init = if matches!(self.current, Token::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(Token::Semicolon)?;

        let cond = if matches!(self.current, Token::Semicolon) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(Token::Semicolon)?;

        let update = if matches!(self.current, Token::RParen) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(Token::RParen)?;
        self.skip_newlines()?;
        let body = Box::new(self.parse_stmt()?);

        Ok(Stmt::For {
            init,
            cond,
            update,
            body,
        })
    }

    fn parse_assignment_from_var(&mut self, var_name: String) -> Result<Expr, ParseError> {
        let var = Expr::Var(var_name);

        let op = match &self.current {
            Token::Assign => AssignOp::Assign,
            Token::PlusAssign => AssignOp::AddAssign,
            Token::MinusAssign => AssignOp::SubAssign,
            Token::StarAssign => AssignOp::MulAssign,
            Token::SlashAssign => AssignOp::DivAssign,
            Token::PercentAssign => AssignOp::ModAssign,
            Token::CaretAssign => AssignOp::PowAssign,
            _ => return Ok(var),
        };
        self.advance()?;

        let value = self.parse_expr()?;
        Ok(Expr::Assign {
            target: Box::new(var),
            op,
            value: Box::new(value),
        })
    }

    fn parse_exit(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::Exit)?;
        let value = if self.is_expr_start() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Stmt::Exit(value))
    }

    fn parse_return(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::Return)?;
        let value = if self.is_expr_start() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Stmt::Return(value))
    }

    fn parse_delete(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::Delete)?;
        let array = match &self.current {
            Token::Identifier(n) => n.clone(),
            _ => return Err(self.error("Expected array name")),
        };
        self.advance()?;

        self.expect(Token::LBracket)?;
        let mut indices = vec![self.parse_expr()?];
        while matches!(self.current, Token::Comma) {
            self.advance()?;
            indices.push(self.parse_expr()?);
        }
        self.expect(Token::RBracket)?;

        Ok(Stmt::Delete { array, indices })
    }

    fn parse_print(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::Print)?;

        let mut args = Vec::new();
        if self.is_print_arg_start() {
            args.push(self.parse_print_expr()?);
            while matches!(self.current, Token::Comma) {
                self.advance()?;
                args.push(self.parse_print_expr()?);
            }
        }

        let output = self.parse_output_redir()?;

        Ok(Stmt::Print { args, output })
    }

    fn parse_printf(&mut self) -> Result<Stmt, ParseError> {
        self.expect(Token::Printf)?;

        let format = self.parse_print_expr()?;
        let mut args = Vec::new();

        while matches!(self.current, Token::Comma) {
            self.advance()?;
            args.push(self.parse_print_expr()?);
        }

        let output = self.parse_output_redir()?;

        Ok(Stmt::Printf {
            format,
            args,
            output,
        })
    }

    fn parse_output_redir(&mut self) -> Result<Option<OutputRedir>, ParseError> {
        match &self.current {
            Token::Gt => {
                self.advance()?;
                Ok(Some(OutputRedir::File(self.parse_expr()?)))
            }
            Token::Append => {
                self.advance()?;
                Ok(Some(OutputRedir::Append(self.parse_expr()?)))
            }
            Token::Pipe => {
                self.advance()?;
                Ok(Some(OutputRedir::Pipe(self.parse_expr()?)))
            }
            _ => Ok(None),
        }
    }

    fn is_expr_start(&self) -> bool {
        matches!(
            self.current,
            Token::Number(_)
                | Token::String(_)
                | Token::Regex(_)
                | Token::Identifier(_)
                | Token::Dollar
                | Token::LParen
                | Token::Not
                | Token::Minus
                | Token::Plus
                | Token::PlusPlus
                | Token::MinusMinus
                | Token::Getline
        )
    }

    fn is_print_arg_start(&self) -> bool {
        self.is_expr_start() && !matches!(self.current, Token::Gt | Token::Append | Token::Pipe)
    }

    /// Parse expression for print (stops at > >> | which are redirects, not operators)
    fn parse_print_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_print_ternary()
    }

    fn parse_print_ternary(&mut self) -> Result<Expr, ParseError> {
        let cond = self.parse_print_or()?;

        if matches!(self.current, Token::Question) {
            self.advance()?;
            let then_expr = self.parse_expr()?;
            self.expect(Token::Colon)?;
            let else_expr = self.parse_expr()?;
            Ok(Expr::Ternary {
                cond: Box::new(cond),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_print_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_and()?;

        while matches!(self.current, Token::Or) {
            self.advance()?;
            let right = self.parse_print_and()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_print_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_in()?;

        while matches!(self.current, Token::And) {
            self.advance()?;
            let right = self.parse_print_in()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_print_in(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_print_match()?;

        if matches!(self.current, Token::In) {
            self.advance()?;
            let array = match &self.current {
                Token::Identifier(n) => n.clone(),
                _ => return Err(self.error("Expected array name")),
            };
            self.advance()?;
            Ok(Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::In,
                right: Box::new(Expr::Var(array)),
            })
        } else {
            Ok(left)
        }
    }

    fn parse_print_match(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_comparison()?;

        loop {
            let op = match &self.current {
                Token::Match => BinOp::Match,
                Token::NotMatch => BinOp::NotMatch,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_print_comparison()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse comparison for print args - does NOT treat > as comparison
    fn parse_print_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_concat()?;

        loop {
            // Note: Token::Gt (>) and Token::Pipe (|) are NOT included here
            // because in print context they are redirects
            let op = match &self.current {
                Token::Lt => BinOp::Lt,
                Token::Le => BinOp::Le,
                Token::Ge => BinOp::Ge,
                Token::Eq => BinOp::Eq,
                Token::Ne => BinOp::Ne,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_print_concat()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse concatenation for print args - stops at | and >
    fn parse_print_concat(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_additive()?;

        while self.is_print_concat_candidate() {
            let right = self.parse_print_additive()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::Concat,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn is_print_concat_candidate(&self) -> bool {
        matches!(
            self.current,
            Token::Number(_)
                | Token::String(_)
                | Token::Identifier(_)
                | Token::Dollar
                | Token::LParen
                | Token::Getline
        )
    }

    fn parse_print_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_multiplicative()?;

        loop {
            let op = match &self.current {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_print_multiplicative()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_print_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_print_power()?;

        loop {
            let op = match &self.current {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_print_power()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_print_power(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_print_unary()?;

        if matches!(self.current, Token::Caret) {
            self.advance()?;
            let exp = self.parse_print_power()?;
            Ok(Expr::BinaryOp {
                left: Box::new(base),
                op: BinOp::Pow,
                right: Box::new(exp),
            })
        } else {
            Ok(base)
        }
    }

    fn parse_print_unary(&mut self) -> Result<Expr, ParseError> {
        match &self.current {
            Token::Not => {
                self.advance()?;
                let expr = self.parse_print_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            Token::Minus => {
                self.advance()?;
                let expr = self.parse_print_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                })
            }
            Token::Plus => {
                self.advance()?;
                self.parse_print_unary()
            }
            Token::PlusPlus => {
                self.advance()?;
                let expr = self.parse_print_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::PreInc,
                    expr: Box::new(expr),
                })
            }
            Token::MinusMinus => {
                self.advance()?;
                let expr = self.parse_print_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::PreDec,
                    expr: Box::new(expr),
                })
            }
            _ => self.parse_print_postfix(),
        }
    }

    /// Postfix for print args - does NOT handle | (it's a redirect in print context)
    fn parse_print_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            match &self.current {
                Token::PlusPlus => {
                    self.advance()?;
                    expr = Expr::UnaryOp {
                        op: UnaryOp::PostInc,
                        expr: Box::new(expr),
                    };
                }
                Token::MinusMinus => {
                    self.advance()?;
                    expr = Expr::UnaryOp {
                        op: UnaryOp::PostDec,
                        expr: Box::new(expr),
                    };
                }
                Token::LBracket => {
                    if let Expr::Var(name) = expr {
                        self.advance()?;
                        let mut indices = vec![self.parse_expr()?];
                        while matches!(self.current, Token::Comma) {
                            self.advance()?;
                            indices.push(self.parse_expr()?);
                        }
                        self.expect(Token::RBracket)?;
                        expr = Expr::ArrayAccess { name, indices };
                    } else {
                        break;
                    }
                }
                // Note: No Token::Pipe here - in print context, | is handled as redirect
                _ => break,
            }
        }

        Ok(expr)
    }

    /// Parse expression
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> Result<Expr, ParseError> {
        let expr = self.parse_ternary()?;

        let op = match &self.current {
            Token::Assign => AssignOp::Assign,
            Token::PlusAssign => AssignOp::AddAssign,
            Token::MinusAssign => AssignOp::SubAssign,
            Token::StarAssign => AssignOp::MulAssign,
            Token::SlashAssign => AssignOp::DivAssign,
            Token::PercentAssign => AssignOp::ModAssign,
            Token::CaretAssign => AssignOp::PowAssign,
            _ => return Ok(expr),
        };
        self.advance()?;

        let value = self.parse_assignment()?;
        Ok(Expr::Assign {
            target: Box::new(expr),
            op,
            value: Box::new(value),
        })
    }

    fn parse_ternary(&mut self) -> Result<Expr, ParseError> {
        let cond = self.parse_or()?;

        if matches!(self.current, Token::Question) {
            self.advance()?;
            let then_expr = self.parse_expr()?;
            self.expect(Token::Colon)?;
            let else_expr = self.parse_expr()?;
            Ok(Expr::Ternary {
                cond: Box::new(cond),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;

        while matches!(self.current, Token::Or) {
            self.advance()?;
            let right = self.parse_and()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_in()?;

        while matches!(self.current, Token::And) {
            self.advance()?;
            let right = self.parse_in()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_in(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_match()?;

        if matches!(self.current, Token::In) {
            self.advance()?;
            let array = match &self.current {
                Token::Identifier(n) => n.clone(),
                _ => return Err(self.error("Expected array name")),
            };
            self.advance()?;
            Ok(Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::In,
                right: Box::new(Expr::Var(array)),
            })
        } else {
            Ok(left)
        }
    }

    fn parse_match(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;

        loop {
            let op = match &self.current {
                Token::Match => BinOp::Match,
                Token::NotMatch => BinOp::NotMatch,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_concat()?;

        loop {
            let op = match &self.current {
                Token::Lt => BinOp::Lt,
                Token::Le => BinOp::Le,
                Token::Gt => BinOp::Gt,
                Token::Ge => BinOp::Ge,
                Token::Eq => BinOp::Eq,
                Token::Ne => BinOp::Ne,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_concat()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_concat(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_additive()?;

        // String concatenation happens when two expressions are adjacent
        // This is tricky - we check if next token starts an expression
        while self.is_concat_candidate() {
            let right = self.parse_additive()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::Concat,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn is_concat_candidate(&self) -> bool {
        matches!(
            self.current,
            Token::Number(_)
                | Token::String(_)
                | Token::Identifier(_)
                | Token::Dollar
                | Token::LParen
                | Token::Getline
        )
    }

    fn parse_additive(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_multiplicative()?;

        loop {
            let op = match &self.current {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_multiplicative()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_power()?;

        loop {
            let op = match &self.current {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance()?;
            let right = self.parse_power()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_power(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_unary()?;

        if matches!(self.current, Token::Caret) {
            self.advance()?;
            let exp = self.parse_power()?; // Right associative
            Ok(Expr::BinaryOp {
                left: Box::new(base),
                op: BinOp::Pow,
                right: Box::new(exp),
            })
        } else {
            Ok(base)
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match &self.current {
            Token::Not => {
                self.advance()?;
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            Token::Minus => {
                self.advance()?;
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                })
            }
            Token::Plus => {
                self.advance()?;
                self.parse_unary()
            }
            Token::PlusPlus => {
                self.advance()?;
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::PreInc,
                    expr: Box::new(expr),
                })
            }
            Token::MinusMinus => {
                self.advance()?;
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::PreDec,
                    expr: Box::new(expr),
                })
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;

        loop {
            match &self.current {
                Token::PlusPlus => {
                    self.advance()?;
                    expr = Expr::UnaryOp {
                        op: UnaryOp::PostInc,
                        expr: Box::new(expr),
                    };
                }
                Token::MinusMinus => {
                    self.advance()?;
                    expr = Expr::UnaryOp {
                        op: UnaryOp::PostDec,
                        expr: Box::new(expr),
                    };
                }
                Token::LBracket => {
                    if let Expr::Var(name) = expr {
                        self.advance()?;
                        let mut indices = vec![self.parse_expr()?];
                        while matches!(self.current, Token::Comma) {
                            self.advance()?;
                            indices.push(self.parse_expr()?);
                        }
                        self.expect(Token::RBracket)?;
                        expr = Expr::ArrayAccess { name, indices };
                    } else {
                        break;
                    }
                }
                Token::Pipe => {
                    // Check for cmd | getline pattern
                    self.advance()?;
                    if matches!(self.current, Token::Getline) {
                        self.advance()?;

                        // Optional variable name
                        let var = if let Token::Identifier(v) = &self.current {
                            let v = v.clone();
                            self.advance()?;
                            Some(v)
                        } else {
                            None
                        };

                        expr = Expr::Getline {
                            var,
                            file: None,
                            command: Some(Box::new(expr)),
                        };
                    } else {
                        // Not a getline - put the pipe token back conceptually
                        // by returning what we have (for print context, | is a redirect)
                        // This is a limitation - we can't "unread" the token
                        // So we need to handle this differently
                        return Err(
                            self.error("Unexpected '|' - use 'cmd | getline' or print redirect")
                        );
                    }
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match &self.current {
            Token::Number(n) => {
                let n = *n;
                self.advance()?;
                Ok(Expr::Number(n))
            }
            Token::String(s) => {
                let s = s.clone();
                self.advance()?;
                Ok(Expr::String(s))
            }
            Token::Regex(r) => {
                let r = r.clone();
                self.advance()?;
                Ok(Expr::Regex(r))
            }
            Token::Dollar => {
                self.advance()?;
                let expr = self.parse_unary()?;
                Ok(Expr::Field(Box::new(expr)))
            }
            Token::LParen => {
                self.advance()?;
                let expr = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(expr)
            }
            Token::Identifier(name) => {
                let name = name.clone();
                self.advance()?;

                // Check for function call
                if matches!(self.current, Token::LParen) {
                    self.advance()?;
                    let mut args = Vec::new();
                    if !matches!(self.current, Token::RParen) {
                        args.push(self.parse_expr()?);
                        while matches!(self.current, Token::Comma) {
                            self.advance()?;
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(Token::RParen)?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Token::Getline => {
                self.advance()?;

                let var = if let Token::Identifier(v) = &self.current {
                    let v = v.clone();
                    self.advance()?;
                    Some(v)
                } else {
                    None
                };

                // Check for < file
                let file = if matches!(self.current, Token::Lt) {
                    self.advance()?;
                    Some(Box::new(self.parse_unary()?))
                } else {
                    None
                };

                Ok(Expr::Getline {
                    var,
                    file,
                    command: None,
                })
            }
            _ => Err(self.error(&format!("Unexpected token: {:?}", self.current))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_rule() {
        let mut parser = Parser::new("{ print $1 }").unwrap();
        let program = parser.parse().unwrap();
        assert_eq!(program.rules.len(), 1);
    }

    #[test]
    fn test_begin_end() {
        let mut parser = Parser::new("BEGIN { x = 1 } END { print x }").unwrap();
        let program = parser.parse().unwrap();
        assert_eq!(program.rules.len(), 2);
    }

    #[test]
    fn test_regex_pattern() {
        let mut parser = Parser::new("/test/ { print }").unwrap();
        let program = parser.parse().unwrap();
        assert_eq!(program.rules.len(), 1);
    }
}
