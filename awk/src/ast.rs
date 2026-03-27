/// AWK Abstract Syntax Tree definitions
use std::fmt;

/// Binary operators
#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Match,    // ~
    NotMatch, // !~
    And,
    Or,
    In,     // array membership
    Concat, // string concatenation (implicit)
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinOp::Add => write!(f, "+"),
            BinOp::Sub => write!(f, "-"),
            BinOp::Mul => write!(f, "*"),
            BinOp::Div => write!(f, "/"),
            BinOp::Mod => write!(f, "%"),
            BinOp::Pow => write!(f, "^"),
            BinOp::Eq => write!(f, "=="),
            BinOp::Ne => write!(f, "!="),
            BinOp::Lt => write!(f, "<"),
            BinOp::Le => write!(f, "<="),
            BinOp::Gt => write!(f, ">"),
            BinOp::Ge => write!(f, ">="),
            BinOp::Match => write!(f, "~"),
            BinOp::NotMatch => write!(f, "!~"),
            BinOp::And => write!(f, "&&"),
            BinOp::Or => write!(f, "||"),
            BinOp::In => write!(f, "in"),
            BinOp::Concat => write!(f, " "),
        }
    }
}

/// Unary operators
#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

/// Assignment operators
#[derive(Debug, Clone, PartialEq)]
pub enum AssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    PowAssign,
}

/// Expressions
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal
    Number(f64),
    /// String literal
    String(String),
    /// Regex literal (used in patterns and match expressions)
    Regex(String),
    /// Variable reference
    Var(String),
    /// Field access: $expr
    Field(Box<Expr>),
    /// Array access: arr[key] or arr[k1, k2, ...]
    ArrayAccess { name: String, indices: Vec<Expr> },
    /// Binary operation
    BinaryOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    /// Unary operation
    UnaryOp { op: UnaryOp, expr: Box<Expr> },
    /// Ternary conditional: cond ? then : else
    Ternary {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    /// Assignment
    Assign {
        target: Box<Expr>,
        op: AssignOp,
        value: Box<Expr>,
    },
    /// Function call
    Call { name: String, args: Vec<Expr> },
    /// Getline variations
    Getline {
        var: Option<String>,
        file: Option<Box<Expr>>,
        command: Option<Box<Expr>>,
    },
}

/// Output redirection
#[derive(Debug, Clone, PartialEq)]
pub enum OutputRedir {
    /// > file
    File(Expr),
    /// >> file
    Append(Expr),
    /// | command
    Pipe(Expr),
}

/// Statements
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// Expression statement
    Expr(Expr),

    /// Print statement: print expr, expr, ... [> file]
    Print {
        args: Vec<Expr>,
        output: Option<OutputRedir>,
    },

    /// Printf statement: printf format, expr, ... [> file]
    Printf {
        format: Expr,
        args: Vec<Expr>,
        output: Option<OutputRedir>,
    },

    /// If statement
    If {
        cond: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },

    /// While loop
    While { cond: Expr, body: Box<Stmt> },

    /// Do-while loop
    DoWhile { body: Box<Stmt>, cond: Expr },

    /// For loop: for (init; cond; update) body
    For {
        init: Option<Expr>,
        cond: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },

    /// For-in loop: for (var in array) body
    ForIn {
        var: String,
        array: String,
        body: Box<Stmt>,
    },

    /// Block of statements
    Block(Vec<Stmt>),

    /// Break statement
    Break,

    /// Continue statement
    Continue,

    /// Next statement (skip to next record)
    Next,

    /// Nextfile statement (skip to next input file)
    NextFile,

    /// Exit statement
    Exit(Option<Expr>),

    /// Return statement
    Return(Option<Expr>),

    /// Delete statement: delete arr[key]
    Delete { array: String, indices: Vec<Expr> },

    /// Empty statement
    Empty,
}

/// Pattern for rules
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// BEGIN pattern
    Begin,
    /// END pattern
    End,
    /// Expression pattern (true if expr is non-zero/non-empty)
    Expr(Expr),
    /// Range pattern: pat1, pat2
    Range {
        start: Box<Pattern>,
        end: Box<Pattern>,
    },
}

/// A rule consists of a pattern and an action
#[derive(Debug, Clone)]
pub struct Rule {
    pub pattern: Option<Pattern>,
    pub action: Vec<Stmt>,
}

/// User-defined function
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

/// Complete AWK program
#[derive(Debug)]
pub struct Program {
    pub functions: Vec<Function>,
    pub rules: Vec<Rule>,
}

impl Program {
    pub fn new() -> Self {
        Program {
            functions: Vec::new(),
            rules: Vec::new(),
        }
    }
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}
