/// AWK Interpreter
use crate::ast::*;
use crate::builtins::{format_string, get_builtins, BuiltinContext, BuiltinFn};
use crate::regex_compat;
use crate::value::{compare_values, Value, Variables};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

/// Runtime error
#[derive(Debug)]
pub struct RuntimeError {
    pub message: String,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Runtime error: {}", self.message)
    }
}

/// Control flow signals
#[derive(Debug)]
enum ControlFlow {
    None,
    Break,
    Continue,
    Next,
    NextFile,
    Exit(i32),
    Return(Value),
}

/// Range pattern state
#[derive(Default)]
struct RangeState {
    active: bool,
}

/// Output file handle wrapper
enum OutputHandle {
    File(File),
    Pipe(Child, ChildStdin),
}

/// Input source type
enum InputSource {
    File(RecordReader),
    Pipe(RecordReader),
}

/// Input file handle wrapper  
struct InputHandle {
    source: InputSource,
}

struct RecordReader {
    content: String,
    position: usize,
}

impl RecordReader {
    fn from_read<R: Read>(mut input: R) -> Result<Self, RuntimeError> {
        let mut content = String::new();
        input
            .read_to_string(&mut content)
            .map_err(|e| RuntimeError {
                message: e.to_string(),
            })?;
        Ok(Self {
            content: normalize_input(&content),
            position: 0,
        })
    }

    fn next_record(&mut self, rs: &str) -> Option<String> {
        if self.position >= self.content.len() {
            return None;
        }

        if rs.is_empty() {
            return self.next_paragraph();
        }

        let sep = rs.chars().next().unwrap_or('\n');
        if let Some(rel_end) = self.content[self.position..].find(sep) {
            let end = self.position + rel_end;
            let record = self.content[self.position..end].to_string();
            self.position = end + sep.len_utf8();
            Some(record)
        } else {
            let record = self.content[self.position..].to_string();
            self.position = self.content.len();
            Some(record)
        }
    }

    fn next_paragraph(&mut self) -> Option<String> {
        let bytes = self.content.as_bytes();
        while self.position < bytes.len() && bytes[self.position] == b'\n' {
            self.position += 1;
        }
        if self.position >= bytes.len() {
            return None;
        }

        let start = self.position;
        let mut idx = self.position;
        while idx < bytes.len() {
            if bytes[idx] == b'\n' {
                let mut next = idx + 1;
                while next < bytes.len() && bytes[next] == b'\n' {
                    next += 1;
                }
                if next > idx + 1 {
                    let record = self.content[start..idx].to_string();
                    self.position = next;
                    return Some(record);
                }
            }
            idx += 1;
        }

        let record = self.content[start..].trim_end_matches('\n').to_string();
        self.position = bytes.len();
        Some(record)
    }
}

/// AWK Interpreter
pub struct Interpreter<'a> {
    program: &'a Program,
    variables: Variables,

    // Built-in variables
    nr: i64,
    fnr: i64,
    nf: i64,
    fs: String,
    rs: String,
    ofs: String,
    ors: String,
    ofmt: String,
    subsep: String,
    filename: String,

    // Current record and fields
    record: String,
    fields: Vec<String>,

    // User-defined functions
    functions: HashMap<String, &'a Function>,

    // Built-in functions
    builtins: HashMap<&'static str, BuiltinFn>,

    // Range pattern states
    range_states: Vec<RangeState>,

    // Random number generator state
    rng_state: u64,

    // Output writer (stdout)
    output: Box<dyn Write>,

    // Output file handles for redirection
    output_files: HashMap<String, OutputHandle>,

    // Input file handles for getline
    input_files: HashMap<String, InputHandle>,

    // Current input stream for plain getline
    current_input: Option<RecordReader>,

    // Exit code
    exit_code: i32,

    begin_executed: bool,
}

impl<'a> Interpreter<'a> {
    pub fn new(program: &'a Program, output: Box<dyn Write>) -> Self {
        let mut functions = HashMap::new();
        for func in &program.functions {
            functions.insert(func.name.clone(), func);
        }

        let range_count = program
            .rules
            .iter()
            .filter(|r| matches!(r.pattern, Some(Pattern::Range { .. })))
            .count();

        Interpreter {
            program,
            variables: Variables::new(),
            nr: 0,
            fnr: 0,
            nf: 0,
            fs: " ".to_string(),
            rs: "\n".to_string(),
            ofs: " ".to_string(),
            ors: "\n".to_string(),
            ofmt: "%.6g".to_string(),
            subsep: "\x1c".to_string(),
            filename: String::new(),
            record: String::new(),
            fields: Vec::new(),
            functions,
            builtins: get_builtins(),
            range_states: (0..range_count).map(|_| RangeState::default()).collect(),
            rng_state: 1,
            output,
            output_files: HashMap::new(),
            input_files: HashMap::new(),
            current_input: None,
            exit_code: 0,
            begin_executed: false,
        }
    }

    /// Set a variable before execution (e.g., from -v option)
    pub fn set_var(&mut self, name: &str, value: &str) {
        match name {
            "FS" => self.fs = value.to_string(),
            "OFS" => self.ofs = value.to_string(),
            "ORS" => self.ors = value.to_string(),
            "RS" => self.rs = value.to_string(),
            "OFMT" => self.ofmt = value.to_string(),
            "SUBSEP" => self.subsep = value.to_string(),
            _ => self
                .variables
                .set(name, Value::from_string(value.to_string())),
        }
    }

    pub fn set_argv(&mut self, argv: &[String]) {
        self.variables.set("ARGC", Value::Number(argv.len() as f64));
        self.variables
            .arrays
            .insert("ARGV".to_string(), HashMap::new());
        for (idx, arg) in argv.iter().enumerate() {
            self.variables
                .set_array("ARGV", &idx.to_string(), Value::from_string(arg.clone()));
        }
    }

    pub fn set_environ<I>(&mut self, env_vars: I)
    where
        I: IntoIterator<Item = (String, String)>,
    {
        self.variables
            .arrays
            .insert("ENVIRON".to_string(), HashMap::new());
        for (key, value) in env_vars {
            self.variables
                .set_array("ENVIRON", &key, Value::from_string(value));
        }
    }

    /// Run the AWK program
    pub fn run<R: Read>(&mut self, input: R, filename: &str) -> Result<i32, RuntimeError> {
        self.filename = filename.to_string();
        self.fnr = 0;

        if let Some(code) = self.run_begin_rules()? {
            return Ok(code);
        }

        self.current_input = Some(RecordReader::from_read(input)?);

        // Process each record
        while let Some(line) = self.read_next_main_record() {
            self.nr += 1;
            self.fnr += 1;
            self.set_record(&line);

            // Run main rules
            let mut range_idx = 0;
            for rule in &self.program.rules {
                match &rule.pattern {
                    Some(Pattern::Begin) | Some(Pattern::End) => continue,
                    Some(Pattern::Range { start, end }) => {
                        let should_run = self.eval_range_pattern(start, end, range_idx)?;
                        range_idx += 1;
                        if !should_run {
                            continue;
                        }
                    }
                    Some(pattern) => {
                        if !self.eval_pattern(pattern)? {
                            continue;
                        }
                    }
                    None => {}
                }

                match self.execute_action(&rule.action)? {
                    ControlFlow::Next => break,
                    ControlFlow::NextFile => {
                        self.current_input = None;
                        return Ok(0);
                    }
                    ControlFlow::Exit(code) => {
                        self.exit_code = code;
                        self.run_end_rules()?;
                        return Ok(code);
                    }
                    _ => {}
                }
            }
        }

        self.current_input = None;

        Ok(0)
    }

    pub fn run_begin_rules(&mut self) -> Result<Option<i32>, RuntimeError> {
        if self.begin_executed {
            return Ok(None);
        }

        self.begin_executed = true;
        for rule in &self.program.rules {
            if matches!(rule.pattern, Some(Pattern::Begin)) {
                if let ControlFlow::Exit(code) = self.execute_action(&rule.action)? {
                    self.exit_code = code;
                    return Ok(Some(code));
                }
            }
        }

        Ok(None)
    }

    /// Run END rules
    pub fn run_end_rules(&mut self) -> Result<(), RuntimeError> {
        for rule in &self.program.rules {
            if matches!(rule.pattern, Some(Pattern::End)) {
                match self.execute_action(&rule.action)? {
                    ControlFlow::Exit(code) => {
                        self.exit_code = code;
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Get the exit code
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn input_file_args(&self) -> Vec<String> {
        let argc = self.variables.get("ARGC").to_number().max(0.0) as usize;
        (1..argc)
            .filter_map(|idx| {
                let value = self.variables.get_array("ARGV", &idx.to_string());
                let text = value.to_string();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            })
            .collect()
    }

    fn set_record(&mut self, record: &str) {
        self.record = record.to_string();
        self.split_record();
    }

    fn split_record(&mut self) {
        self.fields = if self.fs == " " {
            self.record
                .split_whitespace()
                .map(|s| s.to_string())
                .collect()
        } else if self.fs.is_empty() {
            self.record.chars().map(|c| c.to_string()).collect()
        } else if self.fs.len() == 1 {
            self.record.split(&self.fs).map(|s| s.to_string()).collect()
        } else {
            match regex_compat::compile(&self.fs) {
                Ok(re) => re
                    .split(&self.record)
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect(),
                Err(_) => self.record.split(&self.fs).map(|s| s.to_string()).collect(),
            }
        };
        self.nf = self.fields.len() as i64;
    }

    fn rebuild_record(&mut self) {
        self.record = self.fields.join(&self.ofs);
    }

    fn eval_pattern(&mut self, pattern: &Pattern) -> Result<bool, RuntimeError> {
        match pattern {
            Pattern::Begin | Pattern::End => Ok(false),
            Pattern::Expr(expr) => {
                if let Expr::Regex(re) = expr {
                    self.match_regex(re, &self.record.clone())
                } else {
                    let val = self.eval_expr(expr)?;
                    Ok(val.to_bool())
                }
            }
            Pattern::Range { .. } => Ok(false),
        }
    }

    fn eval_range_pattern(
        &mut self,
        start: &Pattern,
        end: &Pattern,
        idx: usize,
    ) -> Result<bool, RuntimeError> {
        let is_active = self.range_states[idx].active;

        if !is_active {
            if self.eval_pattern(start)? {
                self.range_states[idx].active = true;
                return Ok(true);
            }
            Ok(false)
        } else {
            if self.eval_pattern(end)? {
                self.range_states[idx].active = false;
            }
            Ok(true)
        }
    }

    fn match_regex(&self, pattern: &str, text: &str) -> Result<bool, RuntimeError> {
        let re = regex_compat::compile(pattern).map_err(|e| RuntimeError {
            message: format!("Invalid regex: {}", e),
        })?;
        Ok(re.is_match(text))
    }

    fn execute_action(&mut self, stmts: &[Stmt]) -> Result<ControlFlow, RuntimeError> {
        for stmt in stmts {
            match self.execute_stmt(stmt)? {
                ControlFlow::None => {}
                cf => return Ok(cf),
            }
        }
        Ok(ControlFlow::None)
    }

    fn execute_stmt(&mut self, stmt: &Stmt) -> Result<ControlFlow, RuntimeError> {
        match stmt {
            Stmt::Expr(expr) => {
                self.eval_expr(expr)?;
                Ok(ControlFlow::None)
            }

            Stmt::Print { args, output } => {
                let text = if args.is_empty() {
                    self.record.clone()
                } else {
                    args.iter()
                        .map(|e| {
                            let v = self.eval_expr(e)?;
                            Ok(v.to_string_with_ofmt(&self.ofmt))
                        })
                        .collect::<Result<Vec<_>, RuntimeError>>()?
                        .join(&self.ofs)
                };

                self.write_output(&text, output)?;
                Ok(ControlFlow::None)
            }

            Stmt::Printf {
                format,
                args,
                output,
            } => {
                let fmt = self.eval_expr(format)?.to_string();
                let values: Vec<Value> = args
                    .iter()
                    .map(|e| self.eval_expr(e))
                    .collect::<Result<_, _>>()?;
                let text = format_string(&fmt, &values);

                self.write_output_raw(&text, output)?;
                Ok(ControlFlow::None)
            }

            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let cond_val = self.eval_expr(cond)?;
                if cond_val.to_bool() {
                    self.execute_stmt(then_branch)
                } else if let Some(else_stmt) = else_branch {
                    self.execute_stmt(else_stmt)
                } else {
                    Ok(ControlFlow::None)
                }
            }

            Stmt::While { cond, body } => {
                loop {
                    let cond_val = self.eval_expr(cond)?;
                    if !cond_val.to_bool() {
                        break;
                    }
                    match self.execute_stmt(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                        ControlFlow::Next => return Ok(ControlFlow::Next),
                        ControlFlow::NextFile => return Ok(ControlFlow::NextFile),
                        ControlFlow::Exit(code) => return Ok(ControlFlow::Exit(code)),
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::None => {}
                    }
                }
                Ok(ControlFlow::None)
            }

            Stmt::DoWhile { body, cond } => {
                loop {
                    match self.execute_stmt(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => {}
                        ControlFlow::Next => return Ok(ControlFlow::Next),
                        ControlFlow::NextFile => return Ok(ControlFlow::NextFile),
                        ControlFlow::Exit(code) => return Ok(ControlFlow::Exit(code)),
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::None => {}
                    }
                    let cond_val = self.eval_expr(cond)?;
                    if !cond_val.to_bool() {
                        break;
                    }
                }
                Ok(ControlFlow::None)
            }

            Stmt::For {
                init,
                cond,
                update,
                body,
            } => {
                if let Some(init_expr) = init {
                    self.eval_expr(init_expr)?;
                }
                loop {
                    if let Some(cond_expr) = cond {
                        let cond_val = self.eval_expr(cond_expr)?;
                        if !cond_val.to_bool() {
                            break;
                        }
                    }
                    match self.execute_stmt(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => {}
                        ControlFlow::Next => return Ok(ControlFlow::Next),
                        ControlFlow::NextFile => return Ok(ControlFlow::NextFile),
                        ControlFlow::Exit(code) => return Ok(ControlFlow::Exit(code)),
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::None => {}
                    }
                    if let Some(update_expr) = update {
                        self.eval_expr(update_expr)?;
                    }
                }
                Ok(ControlFlow::None)
            }

            Stmt::ForIn { var, array, body } => {
                let keys = self.variables.array_keys(array);
                for key in keys {
                    self.variables.set(var, Value::String(key));
                    match self.execute_stmt(body)? {
                        ControlFlow::Break => break,
                        ControlFlow::Continue => continue,
                        ControlFlow::Next => return Ok(ControlFlow::Next),
                        ControlFlow::NextFile => return Ok(ControlFlow::NextFile),
                        ControlFlow::Exit(code) => return Ok(ControlFlow::Exit(code)),
                        ControlFlow::Return(v) => return Ok(ControlFlow::Return(v)),
                        ControlFlow::None => {}
                    }
                }
                Ok(ControlFlow::None)
            }

            Stmt::Block(stmts) => self.execute_action(stmts),

            Stmt::Break => Ok(ControlFlow::Break),
            Stmt::Continue => Ok(ControlFlow::Continue),
            Stmt::Next => Ok(ControlFlow::Next),
            Stmt::NextFile => Ok(ControlFlow::NextFile),

            Stmt::Exit(expr) => {
                let code = if let Some(e) = expr {
                    self.eval_expr(e)?.to_number() as i32
                } else {
                    0
                };
                Ok(ControlFlow::Exit(code))
            }

            Stmt::Return(expr) => {
                let val = if let Some(e) = expr {
                    self.eval_expr(e)?
                } else {
                    Value::Uninitialized
                };
                Ok(ControlFlow::Return(val))
            }

            Stmt::Delete { array, indices } => {
                let key = self.make_array_key(indices)?;
                self.variables.delete_array(array, &key);
                Ok(ControlFlow::None)
            }

            Stmt::Empty => Ok(ControlFlow::None),
        }
    }

    fn write_output(
        &mut self,
        text: &str,
        redir: &Option<OutputRedir>,
    ) -> Result<(), RuntimeError> {
        match redir {
            None => writeln!(self.output, "{}", text).map_err(|e| RuntimeError {
                message: e.to_string(),
            }),
            Some(r) => self.write_to_redir(text, r, true),
        }
    }

    fn write_output_raw(
        &mut self,
        text: &str,
        redir: &Option<OutputRedir>,
    ) -> Result<(), RuntimeError> {
        match redir {
            None => write!(self.output, "{}", text).map_err(|e| RuntimeError {
                message: e.to_string(),
            }),
            Some(r) => self.write_to_redir(text, r, false),
        }
    }

    fn write_to_redir(
        &mut self,
        text: &str,
        redir: &OutputRedir,
        newline: bool,
    ) -> Result<(), RuntimeError> {
        let (target, is_append, is_pipe) = match redir {
            OutputRedir::File(expr) => {
                let target = self.eval_expr(expr)?.to_string();
                (target, false, false)
            }
            OutputRedir::Append(expr) => {
                let target = self.eval_expr(expr)?.to_string();
                (target, true, false)
            }
            OutputRedir::Pipe(expr) => {
                let target = self.eval_expr(expr)?.to_string();
                (target, false, true)
            }
        };
        let key = if is_pipe {
            target.clone()
        } else {
            normalize_file_key(&target)
        };

        // Get or create the output handle
        if !self.output_files.contains_key(&key) {
            let handle = if is_pipe {
                self.open_pipe(&target)?
            } else {
                self.open_output_file(&target, is_append)?
            };
            self.output_files.insert(key.clone(), handle);
        }

        let handle = self.output_files.get_mut(&key).unwrap();
        let result = match handle {
            OutputHandle::File(f) => {
                if newline {
                    writeln!(f, "{}", text)
                } else {
                    write!(f, "{}", text)
                }
            }
            OutputHandle::Pipe(_, stdin) => {
                if newline {
                    writeln!(stdin, "{}", text)
                } else {
                    write!(stdin, "{}", text)
                }
            }
        };

        result.map_err(|e| RuntimeError {
            message: e.to_string(),
        })
    }

    fn open_output_file(&self, path: &str, append: bool) -> Result<OutputHandle, RuntimeError> {
        let file = if append {
            OpenOptions::new().create(true).append(true).open(path)
        } else {
            File::create(path)
        };

        file.map(OutputHandle::File).map_err(|e| RuntimeError {
            message: format!("Cannot open '{}': {}", path, e),
        })
    }

    fn open_pipe(&self, command: &str) -> Result<OutputHandle, RuntimeError> {
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

        let mut child = Command::new(shell)
            .arg(shell_arg)
            .arg(command)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| RuntimeError {
                message: format!("Cannot execute '{}': {}", command, e),
            })?;

        let stdin = child.stdin.take().ok_or_else(|| RuntimeError {
            message: "Failed to open pipe stdin".to_string(),
        })?;

        Ok(OutputHandle::Pipe(child, stdin))
    }

    /// Close an output file or pipe
    pub fn close_file(&mut self, name: &str) -> i32 {
        let output_key = normalize_file_key(name);
        if let Some(handle) = self.output_files.remove(&output_key) {
            match handle {
                OutputHandle::File(_) => 0,
                OutputHandle::Pipe(mut child, _) => {
                    child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1)
                }
            }
        } else if let Some(handle) = self.output_files.remove(name) {
            match handle {
                OutputHandle::File(_) => 0,
                OutputHandle::Pipe(mut child, _) => {
                    child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1)
                }
            }
        } else if self.input_files.remove(&output_key).is_some()
            || self.input_files.remove(name).is_some()
        {
            0
        } else {
            -1
        }
    }

    /// Read a line from a file for getline
    fn getline_from_file(&mut self, filename: &str) -> Result<Option<String>, RuntimeError> {
        let key = normalize_file_key(filename);
        if !self.input_files.contains_key(&key) {
            let file = File::open(filename).map_err(|e| RuntimeError {
                message: format!("Cannot open '{}': {}", filename, e),
            })?;
            let reader = RecordReader::from_read(file)?;
            self.input_files.insert(
                key.clone(),
                InputHandle {
                    source: InputSource::File(reader),
                },
            );
        }

        let handle = self.input_files.get_mut(&key).unwrap();
        Ok(match &mut handle.source {
            InputSource::File(reader) | InputSource::Pipe(reader) => reader.next_record(&self.rs),
        })
    }

    /// Read a line from a command pipe for getline
    fn getline_from_command(&mut self, command: &str) -> Result<Option<String>, RuntimeError> {
        // Use command as key, but prefix with "|" to distinguish from files
        let key = format!("|{}", command);

        if !self.input_files.contains_key(&key) {
            let shell = if cfg!(windows) { "cmd" } else { "sh" };
            let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

            let mut child = Command::new(shell)
                .arg(shell_arg)
                .arg(command)
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|e| RuntimeError {
                    message: format!("Cannot execute '{}': {}", command, e),
                })?;

            let stdout = child.stdout.take().ok_or_else(|| RuntimeError {
                message: "Failed to open pipe stdout".to_string(),
            })?;
            let reader = RecordReader::from_read(stdout)?;

            self.input_files.insert(
                key.clone(),
                InputHandle {
                    source: InputSource::Pipe(reader),
                },
            );
        }

        let handle = self.input_files.get_mut(&key).unwrap();
        Ok(match &mut handle.source {
            InputSource::File(reader) | InputSource::Pipe(reader) => reader.next_record(&self.rs),
        })
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Number(n) => Ok(Value::Number(*n)),
            Expr::String(s) => Ok(Value::String(s.clone())),
            Expr::Regex(r) => {
                let matched = self.match_regex(r, &self.record.clone())?;
                Ok(Value::Number(if matched { 1.0 } else { 0.0 }))
            }

            Expr::Var(name) => Ok(self.get_var(name)),

            Expr::Field(idx_expr) => {
                let idx = self.eval_expr(idx_expr)?.to_number() as i64;
                Ok(self.get_field(idx))
            }

            Expr::ArrayAccess { name, indices } => {
                let key = self.make_array_key(indices)?;
                Ok(self.variables.get_array(name, &key))
            }

            Expr::BinaryOp { left, op, right } => self.eval_binary_op(left, op, right),

            Expr::UnaryOp { op, expr } => self.eval_unary_op(op, expr),

            Expr::Ternary {
                cond,
                then_expr,
                else_expr,
            } => {
                let cond_val = self.eval_expr(cond)?;
                if cond_val.to_bool() {
                    self.eval_expr(then_expr)
                } else {
                    self.eval_expr(else_expr)
                }
            }

            Expr::Assign { target, op, value } => {
                let new_val = self.eval_expr(value)?;
                let final_val = match op {
                    AssignOp::Assign => new_val,
                    _ => {
                        let old_val = self.eval_lvalue(target)?;
                        self.apply_assign_op(op, &old_val, &new_val)
                    }
                };
                self.set_lvalue(target, final_val.clone())?;
                Ok(final_val)
            }

            Expr::Call { name, args } => self.call_function(name, args),

            Expr::Getline { var, file, command } => {
                self.eval_getline(var.as_deref(), file.as_deref(), command.as_deref())
            }
        }
    }

    fn get_var(&self, name: &str) -> Value {
        match name {
            "NR" => Value::Number(self.nr as f64),
            "NF" => Value::Number(self.nf as f64),
            "FNR" => Value::Number(self.fnr as f64),
            "ARGC" => self.variables.get("ARGC"),
            "FS" => Value::String(self.fs.clone()),
            "RS" => Value::String(self.rs.clone()),
            "OFS" => Value::String(self.ofs.clone()),
            "ORS" => Value::String(self.ors.clone()),
            "OFMT" => Value::String(self.ofmt.clone()),
            "FILENAME" => Value::String(self.filename.clone()),
            "SUBSEP" => Value::String(self.subsep.clone()),
            "RSTART" => self.variables.get("RSTART"),
            "RLENGTH" => self.variables.get("RLENGTH"),
            _ => self.variables.get(name),
        }
    }

    fn get_field(&self, idx: i64) -> Value {
        if idx == 0 {
            Value::String(self.record.clone())
        } else if idx > 0 && (idx as usize) <= self.fields.len() {
            Value::from_string(self.fields[idx as usize - 1].clone())
        } else {
            Value::Uninitialized
        }
    }

    fn set_field(&mut self, idx: i64, value: Value) {
        if idx == 0 {
            self.set_record(&value.to_string());
        } else if idx > 0 {
            let idx = idx as usize - 1;
            while self.fields.len() <= idx {
                self.fields.push(String::new());
            }
            self.fields[idx] = value.to_string();
            self.nf = self.fields.len() as i64;
            self.rebuild_record();
        }
    }

    fn eval_lvalue(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Var(name) => Ok(self.get_var(name)),
            Expr::Field(idx) => {
                let idx = self.eval_expr(idx)?.to_number() as i64;
                Ok(self.get_field(idx))
            }
            Expr::ArrayAccess { name, indices } => {
                let key = self.make_array_key(indices)?;
                Ok(self.variables.get_array(name, &key))
            }
            _ => Err(RuntimeError {
                message: "Invalid lvalue".to_string(),
            }),
        }
    }

    fn set_lvalue(&mut self, expr: &Expr, value: Value) -> Result<(), RuntimeError> {
        match expr {
            Expr::Var(name) => {
                match name.as_str() {
                    "NR" => self.nr = value.to_number() as i64,
                    "NF" => {
                        let new_nf = value.to_number() as usize;
                        self.fields.resize(new_nf, String::new());
                        self.nf = new_nf as i64;
                        self.rebuild_record();
                    }
                    "ARGC" => self.variables.set("ARGC", value),
                    "FS" => self.fs = value.to_string(),
                    "RS" => self.rs = value.to_string(),
                    "OFS" => self.ofs = value.to_string(),
                    "ORS" => self.ors = value.to_string(),
                    "OFMT" => self.ofmt = value.to_string(),
                    "RSTART" | "RLENGTH" => self.variables.set(name, value),
                    _ => self.variables.set(name, value),
                }
                Ok(())
            }
            Expr::Field(idx_expr) => {
                let idx = self.eval_expr(idx_expr)?.to_number() as i64;
                self.set_field(idx, value);
                Ok(())
            }
            Expr::ArrayAccess { name, indices } => {
                let key = self.make_array_key(indices)?;
                self.variables.set_array(name, &key, value);
                Ok(())
            }
            _ => Err(RuntimeError {
                message: "Invalid lvalue".to_string(),
            }),
        }
    }

    fn make_array_key(&mut self, indices: &[Expr]) -> Result<String, RuntimeError> {
        let parts: Vec<String> = indices
            .iter()
            .map(|e| self.eval_expr(e).map(|v| v.to_string()))
            .collect::<Result<_, _>>()?;
        Ok(parts.join(&self.subsep))
    }

    fn apply_assign_op(&self, op: &AssignOp, old: &Value, new: &Value) -> Value {
        let old_num = old.to_number();
        let new_num = new.to_number();
        Value::Number(match op {
            AssignOp::Assign => new_num,
            AssignOp::AddAssign => old_num + new_num,
            AssignOp::SubAssign => old_num - new_num,
            AssignOp::MulAssign => old_num * new_num,
            AssignOp::DivAssign => old_num / new_num,
            AssignOp::ModAssign => old_num % new_num,
            AssignOp::PowAssign => old_num.powf(new_num),
        })
    }

    fn eval_binary_op(
        &mut self,
        left: &Expr,
        op: &BinOp,
        right: &Expr,
    ) -> Result<Value, RuntimeError> {
        match op {
            BinOp::And => {
                let l = self.eval_expr(left)?;
                if !l.to_bool() {
                    return Ok(Value::Number(0.0));
                }
                let r = self.eval_expr(right)?;
                return Ok(Value::Number(if r.to_bool() { 1.0 } else { 0.0 }));
            }
            BinOp::Or => {
                let l = self.eval_expr(left)?;
                if l.to_bool() {
                    return Ok(Value::Number(1.0));
                }
                let r = self.eval_expr(right)?;
                return Ok(Value::Number(if r.to_bool() { 1.0 } else { 0.0 }));
            }
            BinOp::Match => {
                let l = self.eval_expr(left)?;
                let text = l.to_string();
                // If right side is a regex literal, use it directly without evaluating
                let pattern = if let Expr::Regex(re) = right {
                    re.clone()
                } else {
                    self.eval_expr(right)?.to_string()
                };
                let matched = self.match_regex(&pattern, &text)?;
                return Ok(Value::Number(if matched { 1.0 } else { 0.0 }));
            }
            BinOp::NotMatch => {
                let l = self.eval_expr(left)?;
                let text = l.to_string();
                let pattern = if let Expr::Regex(re) = right {
                    re.clone()
                } else {
                    self.eval_expr(right)?.to_string()
                };
                let matched = self.match_regex(&pattern, &text)?;
                return Ok(Value::Number(if matched { 0.0 } else { 1.0 }));
            }
            _ => {}
        }

        let l = self.eval_expr(left)?;
        let r = self.eval_expr(right)?;

        match op {
            BinOp::Add => Ok(Value::Number(l.to_number() + r.to_number())),
            BinOp::Sub => Ok(Value::Number(l.to_number() - r.to_number())),
            BinOp::Mul => Ok(Value::Number(l.to_number() * r.to_number())),
            BinOp::Div => Ok(Value::Number(l.to_number() / r.to_number())),
            BinOp::Mod => Ok(Value::Number(l.to_number() % r.to_number())),
            BinOp::Pow => Ok(Value::Number(l.to_number().powf(r.to_number()))),

            BinOp::Eq => {
                let cmp = compare_values(&l, &r);
                Ok(Value::Number(if cmp == std::cmp::Ordering::Equal {
                    1.0
                } else {
                    0.0
                }))
            }
            BinOp::Ne => {
                let cmp = compare_values(&l, &r);
                Ok(Value::Number(if cmp != std::cmp::Ordering::Equal {
                    1.0
                } else {
                    0.0
                }))
            }
            BinOp::Lt => {
                let cmp = compare_values(&l, &r);
                Ok(Value::Number(if cmp == std::cmp::Ordering::Less {
                    1.0
                } else {
                    0.0
                }))
            }
            BinOp::Le => {
                let cmp = compare_values(&l, &r);
                Ok(Value::Number(if cmp != std::cmp::Ordering::Greater {
                    1.0
                } else {
                    0.0
                }))
            }
            BinOp::Gt => {
                let cmp = compare_values(&l, &r);
                Ok(Value::Number(if cmp == std::cmp::Ordering::Greater {
                    1.0
                } else {
                    0.0
                }))
            }
            BinOp::Ge => {
                let cmp = compare_values(&l, &r);
                Ok(Value::Number(if cmp != std::cmp::Ordering::Less {
                    1.0
                } else {
                    0.0
                }))
            }

            BinOp::Concat => Ok(Value::String(format!("{}{}", l.to_string(), r.to_string()))),

            BinOp::In => {
                if let Expr::Var(array) = right {
                    let key = l.to_string();
                    let exists = self.variables.has_array_key(array, &key);
                    Ok(Value::Number(if exists { 1.0 } else { 0.0 }))
                } else {
                    Err(RuntimeError {
                        message: "'in' requires array name".to_string(),
                    })
                }
            }

            BinOp::And | BinOp::Or | BinOp::Match | BinOp::NotMatch => unreachable!(),
        }
    }

    fn eval_unary_op(&mut self, op: &UnaryOp, expr: &Expr) -> Result<Value, RuntimeError> {
        match op {
            UnaryOp::Neg => {
                let v = self.eval_expr(expr)?;
                Ok(Value::Number(-v.to_number()))
            }
            UnaryOp::Not => {
                let v = self.eval_expr(expr)?;
                Ok(Value::Number(if v.to_bool() { 0.0 } else { 1.0 }))
            }
            UnaryOp::PreInc => {
                let old = self.eval_lvalue(expr)?;
                let new = Value::Number(old.to_number() + 1.0);
                self.set_lvalue(expr, new.clone())?;
                Ok(new)
            }
            UnaryOp::PreDec => {
                let old = self.eval_lvalue(expr)?;
                let new = Value::Number(old.to_number() - 1.0);
                self.set_lvalue(expr, new.clone())?;
                Ok(new)
            }
            UnaryOp::PostInc => {
                let old = self.eval_lvalue(expr)?;
                let new = Value::Number(old.to_number() + 1.0);
                self.set_lvalue(expr, new)?;
                Ok(Value::Number(old.to_number()))
            }
            UnaryOp::PostDec => {
                let old = self.eval_lvalue(expr)?;
                let new = Value::Number(old.to_number() - 1.0);
                self.set_lvalue(expr, new)?;
                Ok(Value::Number(old.to_number()))
            }
        }
    }

    fn call_function(&mut self, name: &str, args: &[Expr]) -> Result<Value, RuntimeError> {
        // Handle special functions that need interpreter access
        if name == "close" {
            if args.is_empty() {
                return Err(RuntimeError {
                    message: "close requires 1 argument".to_string(),
                });
            }
            let filename = self.eval_expr(&args[0])?.to_string();
            let result = self.close_file(&filename);
            return Ok(Value::Number(result as f64));
        }
        if name == "fflush" {
            return self.call_fflush(args);
        }
        if name == "system" {
            return self.call_system(args);
        }
        if name == "split" {
            return self.call_split(args);
        }
        if name == "match" {
            return self.call_match(args);
        }
        if name == "sub" {
            return self.call_substitute(args, false);
        }
        if name == "gsub" {
            return self.call_substitute(args, true);
        }

        // First, evaluate all arguments before checking builtins
        let values: Vec<Value> = args
            .iter()
            .map(|e| self.eval_expr(e))
            .collect::<Result<_, _>>()?;

        // Check for builtin function
        if let Some(builtin) = self.builtins.get(name).copied() {
            let mut scalars: HashMap<String, Value> = self.variables.scalars.clone();
            let mut ctx = BuiltinContext {
                record: &self.record,
                fields: &mut self.fields,
                variables: &mut scalars,
                subsep: &self.subsep,
                rng_state: &mut self.rng_state,
            };

            let result = builtin(&values, &mut ctx).map_err(|e| RuntimeError { message: e })?;

            for (k, v) in scalars {
                self.variables.set(&k, v);
            }

            return Ok(result);
        }

        if let Some(func) = self.functions.get(name).cloned() {
            let mut old_values: Vec<(String, Value)> = Vec::new();

            for (i, param) in func.params.iter().enumerate() {
                old_values.push((param.clone(), self.variables.get(param)));
                if i < values.len() {
                    self.variables.set(param, values[i].clone());
                } else {
                    self.variables.set(param, Value::Uninitialized);
                }
            }

            let result = match self.execute_action(&func.body)? {
                ControlFlow::Return(v) => v,
                _ => Value::Uninitialized,
            };

            for (name, val) in old_values {
                self.variables.set(&name, val);
            }

            return Ok(result);
        }

        Err(RuntimeError {
            message: format!("Unknown function: {}", name),
        })
    }

    fn eval_getline(
        &mut self,
        var: Option<&str>,
        file: Option<&Expr>,
        command: Option<&Expr>,
    ) -> Result<Value, RuntimeError> {
        // Determine the source
        let line_result = if let Some(cmd_expr) = command {
            // getline from command: cmd | getline [var]
            let cmd = self.eval_expr(cmd_expr)?.to_string();
            self.getline_from_command(&cmd)?
        } else if let Some(file_expr) = file {
            // getline from file: getline [var] < file
            let filename = self.eval_expr(file_expr)?.to_string();
            self.getline_from_file(&filename)?
        } else {
            self.read_next_main_record()
        };

        match line_result {
            Some(line) => {
                self.nr += 1;
                self.fnr += 1;
                if let Some(var_name) = var {
                    // Store in specified variable
                    self.variables.set(var_name, Value::from_string(line));
                } else {
                    // Update $0 and fields
                    self.set_record(&line);
                }
                Ok(Value::Number(1.0))
            }
            None => Ok(Value::Number(0.0)), // EOF
        }
    }

    fn call_split(&mut self, args: &[Expr]) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError {
                message: "split requires at least 2 arguments".to_string(),
            });
        }

        let source = self.eval_expr(&args[0])?.to_string();
        let array_name = match &args[1] {
            Expr::Var(name) => name.clone(),
            _ => {
                return Err(RuntimeError {
                    message: "split requires an array variable as the second argument".to_string(),
                });
            }
        };
        let separator = if args.len() > 2 {
            self.eval_expr(&args[2])?.to_string()
        } else {
            self.fs.clone()
        };

        let parts = split_with_separator(&source, &separator)?;
        self.variables
            .arrays
            .insert(array_name.clone(), HashMap::new());
        for (idx, part) in parts.iter().enumerate() {
            self.variables.set_array(
                &array_name,
                &(idx + 1).to_string(),
                Value::from_string(part.clone()),
            );
        }

        Ok(Value::Number(parts.len() as f64))
    }

    fn call_fflush(&mut self, args: &[Expr]) -> Result<Value, RuntimeError> {
        if args.len() > 1 {
            return Err(RuntimeError {
                message: "fflush accepts at most 1 argument".to_string(),
            });
        }

        let result = if let Some(arg) = args.first() {
            let target = self.eval_expr(arg)?.to_string();
            self.flush_target(&target)
        } else {
            self.flush_all_outputs()
        };

        Ok(Value::Number(if result.is_ok() { 0.0 } else { -1.0 }))
    }

    fn call_system(&mut self, args: &[Expr]) -> Result<Value, RuntimeError> {
        if args.len() != 1 {
            return Err(RuntimeError {
                message: "system requires 1 argument".to_string(),
            });
        }

        let command = self.eval_expr(&args[0])?.to_string();
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let shell_arg = if cfg!(windows) { "/C" } else { "-c" };
        let status = Command::new(shell)
            .arg(shell_arg)
            .arg(command)
            .status()
            .map_err(|e| RuntimeError {
                message: e.to_string(),
            })?;

        Ok(Value::Number(status.code().unwrap_or(-1) as f64))
    }

    fn call_substitute(&mut self, args: &[Expr], global: bool) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError {
                message: if global {
                    "gsub requires at least 2 arguments".to_string()
                } else {
                    "sub requires at least 2 arguments".to_string()
                },
            });
        }

        let pattern = self.eval_expr(&args[0])?.to_string();
        let replacement = self.eval_expr(&args[1])?.to_string();
        let regex = regex_compat::compile(&pattern).map_err(|e| RuntimeError {
            message: e.to_string(),
        })?;

        let original = if args.len() > 2 {
            self.eval_expr(&args[2])?.to_string()
        } else {
            self.record.clone()
        };

        let (updated, count) = if global {
            substitute_all(&regex, &original, &replacement)
        } else {
            substitute_one(&regex, &original, &replacement)
        };

        if count > 0 {
            if args.len() > 2 {
                self.set_lvalue(&args[2], Value::from_string(updated))?;
            } else {
                self.set_record(&updated);
            }
        }

        Ok(Value::Number(count as f64))
    }

    fn call_match(&mut self, args: &[Expr]) -> Result<Value, RuntimeError> {
        if args.len() < 2 {
            return Err(RuntimeError {
                message: "match requires 2 arguments".to_string(),
            });
        }

        let text = self.eval_expr(&args[0])?.to_string();
        let pattern = self.eval_expr(&args[1])?.to_string();
        let regex = regex_compat::compile(&pattern).map_err(|e| RuntimeError {
            message: e.to_string(),
        })?;

        if let Some(matched) = regex.find(&text) {
            self.variables.set(
                "RSTART",
                Value::Number((text[..matched.start].chars().count() + 1) as f64),
            );
            self.variables.set(
                "RLENGTH",
                Value::Number(text[matched.start..matched.end].chars().count() as f64),
            );
            Ok(Value::Number(
                (text[..matched.start].chars().count() + 1) as f64,
            ))
        } else {
            self.variables.set("RSTART", Value::Number(0.0));
            self.variables.set("RLENGTH", Value::Number(-1.0));
            Ok(Value::Number(0.0))
        }
    }

    fn read_next_main_record(&mut self) -> Option<String> {
        self.current_input
            .as_mut()
            .and_then(|reader| reader.next_record(&self.rs))
    }

    fn flush_all_outputs(&mut self) -> std::io::Result<()> {
        self.output.flush()?;
        for handle in self.output_files.values_mut() {
            match handle {
                OutputHandle::File(file) => file.flush()?,
                OutputHandle::Pipe(_, stdin) => stdin.flush()?,
            }
        }
        Ok(())
    }

    fn flush_target(&mut self, target: &str) -> std::io::Result<()> {
        if target.is_empty() {
            return self.flush_all_outputs();
        }

        let key = normalize_file_key(target);
        if let Some(handle) = self.output_files.get_mut(&key) {
            match handle {
                OutputHandle::File(file) => file.flush(),
                OutputHandle::Pipe(_, stdin) => stdin.flush(),
            }
        } else {
            self.output.flush()
        }
    }
}

fn normalize_file_key(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\").to_ascii_lowercase()
    } else {
        path.to_string()
    }
}

fn normalize_input(input: &str) -> String {
    input.replace("\r\n", "\n").replace('\r', "\n")
}

fn split_with_separator(text: &str, separator: &str) -> Result<Vec<String>, RuntimeError> {
    let parts = if separator == " " {
        text.split_whitespace()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else if separator.is_empty() {
        text.chars().map(|c| c.to_string()).collect::<Vec<_>>()
    } else if separator.len() == 1 {
        text.split(separator)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else {
        let regex = regex_compat::compile(separator).map_err(|e| RuntimeError {
            message: e.to_string(),
        })?;
        regex
            .split(text)
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    };
    Ok(parts)
}

fn substitute_one(
    regex: &regex_compat::PosixRegex,
    input: &str,
    replacement: &str,
) -> (String, usize) {
    if let Some(matched) = regex.find(input) {
        let mut updated = input.to_string();
        let repl = expand_replacement(replacement, &input[matched.start..matched.end]);
        updated.replace_range(matched.start..matched.end, &repl);
        (updated, 1)
    } else {
        (input.to_string(), 0)
    }
}

fn substitute_all(
    regex: &regex_compat::PosixRegex,
    input: &str,
    replacement: &str,
) -> (String, usize) {
    let mut count = 0;
    let mut updated = String::new();
    let mut last_end = 0;
    let mut search_start = 0;

    while let Some(matched) = regex.find_from(input, search_start) {
        updated.push_str(&input[last_end..matched.start]);
        updated.push_str(&expand_replacement(
            replacement,
            &input[matched.start..matched.end],
        ));
        count += 1;

        if matched.start == matched.end {
            if let Some(next_char) = input[matched.end..].chars().next() {
                updated.push(next_char);
                last_end = matched.end + next_char.len_utf8();
                search_start = last_end;
            } else {
                last_end = matched.end;
                break;
            }
        } else {
            last_end = matched.end;
            search_start = matched.end;
        }
    }

    updated.push_str(&input[last_end..]);
    (updated, count)
}

fn expand_replacement(replacement: &str, matched_text: &str) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '&' => out.push_str(matched_text),
            '\\' => {
                if let Some(next) = chars.next() {
                    match next {
                        '&' => out.push('&'),
                        '\\' => out.push('\\'),
                        other => {
                            out.push('\\');
                            out.push(other);
                        }
                    }
                } else {
                    out.push('\\');
                }
            }
            other => out.push(other),
        }
    }

    out
}
