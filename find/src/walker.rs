use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::action::{ActionContext, ActionResult, BatchExecutor, OutputManager};
use crate::expression::{EvalContext, Expression};
use crate::messages;
use crate::parser::{FollowSymlinks, GlobalOptions};
use crate::platform;

/// ウォーカーの状態
pub struct Walker {
    pub options: GlobalOptions,
    pub expression: Option<Expression>,
    pub output_manager: OutputManager,
    pub batch_executor: BatchExecutor,
    pub use_default_print: bool,
    pub exit_code: i32,
    now: SystemTime,
    /// -daystart 用の「今日の終わり（明日 00:00）」
    day_end: SystemTime,
}

impl Walker {
    pub fn new(options: GlobalOptions, expression: Option<Expression>) -> Self {
        let use_default_print = expression
            .as_ref()
            .map(|e| !Self::contains_non_default_action(e))
            .unwrap_or(true);

        let now = SystemTime::now();
        Walker {
            options,
            expression,
            output_manager: OutputManager::new(),
            batch_executor: BatchExecutor::new(),
            use_default_print,
            exit_code: 0,
            now,
            day_end: now,
        }
    }

    fn contains_non_default_action(expr: &Expression) -> bool {
        match expr {
            Expression::Action(action) => !matches!(action, crate::expression::Action::Prune),
            Expression::Test(_) => false,
            Expression::Not(e) => Self::contains_non_default_action(e),
            Expression::And(a, b) | Expression::Or(a, b) | Expression::List(a, b) => {
                Self::contains_non_default_action(a) || Self::contains_non_default_action(b)
            }
        }
    }

    pub fn walk(&mut self, start_paths: &[PathBuf]) {
        self.now = SystemTime::now();
        self.day_end = compute_day_end().unwrap_or(self.now);

        // 式ツリーはファイルごとに clone せず、走査中は take して参照で渡す
        let expression = self.expression.take();

        'paths: for start_path in start_paths {
            // exists() はリンクを辿るため、リンク切れの開始パスも扱えるよう
            // symlink_metadata で存在確認する
            if fs::symlink_metadata(start_path).is_err() {
                eprintln!(
                    "{}",
                    messages::err_path_not_found(&start_path.display().to_string())
                );
                self.exit_code = 1;
                continue;
            }

            let start_dev = if self.options.xdev {
                fs::metadata(start_path)
                    .map(|m| platform::get_dev(&m))
                    .ok()
            } else {
                None
            };

            // -L でのシンボリックリンク循環検出用（走査中の祖先ディレクトリの実体パス）
            let mut ancestors: Vec<PathBuf> = Vec::new();

            match self.visit(start_path, start_path, 0, start_dev, &expression, &mut ancestors) {
                Ok(true) => break 'paths, // -quit は全体を停止する
                Ok(false) => {}
                Err(e) => {
                    eprintln!("{}", e);
                    self.exit_code = 1;
                }
            }
        }

        self.expression = expression;

        self.batch_executor.flush();
        if self.batch_executor.had_failure() {
            self.exit_code = 1;
        }
        self.output_manager.flush_all();
    }

    /// 1エントリを訪問する。戻り値は「-quit したかどうか」。
    fn visit(
        &mut self,
        path: &Path,
        start: &Path,
        depth: usize,
        start_dev: Option<u64>,
        expression: &Option<Expression>,
        ancestors: &mut Vec<PathBuf>,
    ) -> Result<bool, String> {
        let (metadata, symlink_metadata) = match self.get_metadata(path, depth == 0) {
            Some(m) => m,
            None => {
                self.exit_code = 1;
                return Ok(false);
            }
        };

        let is_dir = metadata.is_dir();
        // -xdev: 別ファイルシステムのエントリ自体は評価するが、その中には入らない
        let same_dev = start_dev.map_or(true, |dev| platform::get_dev(&metadata) == dev);

        let mut should_prune = false;

        if !self.options.depth_first {
            let (_matched, prune, quit) = self.evaluate_and_execute(
                path,
                depth,
                start,
                expression,
                &metadata,
                symlink_metadata.as_ref(),
            )?;
            if quit {
                return Ok(true);
            }
            should_prune = prune;
        }

        let should_descend = is_dir
            && !should_prune
            && same_dev
            && self.options.max_depth.map_or(true, |max| depth < max);

        if should_descend {
            // シンボリックリンクを辿ってディレクトリに入る場合は循環を検出する。
            // -L では実ディレクトリも祖先として記録し、循環を最初のリンクで検出する。
            let mut pushed_ancestor = false;
            let mut skip_descend = false;
            if symlink_metadata.is_some()
                || matches!(self.options.follow_symlinks, FollowSymlinks::Always)
            {
                match fs::canonicalize(path) {
                    Ok(canonical) => {
                        if ancestors.contains(&canonical) {
                            eprintln!(
                                "{}",
                                messages::warn_symlink_loop(&path.display().to_string())
                            );
                            skip_descend = true;
                        } else {
                            ancestors.push(canonical);
                            pushed_ancestor = true;
                        }
                    }
                    Err(_) => skip_descend = true,
                }
            }

            if !skip_descend {
                match fs::read_dir(path) {
                    Ok(read_dir) => {
                        for dir_entry in read_dir.flatten() {
                            let quit = self.visit(
                                &dir_entry.path(),
                                start,
                                depth + 1,
                                start_dev,
                                expression,
                                ancestors,
                            )?;
                            if quit {
                                if pushed_ancestor {
                                    ancestors.pop();
                                }
                                return Ok(true);
                            }
                        }
                    }
                    Err(_) => {
                        eprintln!(
                            "{}",
                            messages::warn_cannot_read_dir(&path.display().to_string())
                        );
                        self.exit_code = 1;
                    }
                }
            }

            if pushed_ancestor {
                ancestors.pop();
            }
        }

        if self.options.depth_first {
            let (_matched, _prune, quit) = self.evaluate_and_execute(
                path,
                depth,
                start,
                expression,
                &metadata,
                symlink_metadata.as_ref(),
            )?;
            if quit {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn get_metadata(
        &self,
        path: &Path,
        is_commandline_arg: bool,
    ) -> Option<(Metadata, Option<Metadata>)> {
        let symlink_metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(_e) => {
                eprintln!(
                    "{}",
                    messages::warn_cannot_stat(&path.display().to_string())
                );
                return None;
            }
        };

        let is_symlink = symlink_metadata.file_type().is_symlink();

        let should_follow =
            should_follow_metadata(self.options.follow_symlinks, is_commandline_arg);

        let metadata = if is_symlink && should_follow {
            match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => symlink_metadata.clone(),
            }
        } else {
            symlink_metadata.clone()
        };

        Some((
            metadata,
            if is_symlink {
                Some(symlink_metadata)
            } else {
                None
            },
        ))
    }

    fn evaluate_and_execute(
        &mut self,
        path: &Path,
        depth: usize,
        start: &Path,
        expression: &Option<Expression>,
        metadata: &Metadata,
        symlink_metadata: Option<&Metadata>,
    ) -> Result<(bool, bool, bool), String> {
        if let Some(min) = self.options.min_depth {
            if depth < min {
                return Ok((false, false, false));
            }
        }

        let follow_symlinks = should_follow_metadata(self.options.follow_symlinks, depth == 0);
        let (symlink_meta, followed_meta) = if let Some(sm) = symlink_metadata {
            (sm.clone(), Some(metadata.clone()))
        } else {
            (metadata.clone(), None)
        };
        let mut eval_ctx = EvalContext::new(
            path,
            start,
            depth,
            self.now,
            symlink_meta,
            followed_meta,
            follow_symlinks,
        );
        eval_ctx.day_end = self.day_end;

        let (matched, should_prune, should_quit) = if let Some(expr) = expression {
            self.evaluate_expression(expr, &eval_ctx)?
        } else {
            (true, false, false)
        };

        if matched && self.use_default_print {
            println!("{}", path.display());
        }

        Ok((matched, should_prune, should_quit))
    }

    fn evaluate_expression(
        &mut self,
        expr: &Expression,
        ctx: &EvalContext,
    ) -> Result<(bool, bool, bool), String> {
        match expr {
            Expression::Test(test) => {
                let result = test.evaluate(ctx);
                Ok((result, false, false))
            }

            Expression::Action(action) => {
                // アクションに必要なメタデータを EvalContext から取得。
                // -name/-path など「メタデータ不要な式」の後に続くアクションでも
                // -print/-ls/-printf などは metadata を必要とするため、ここで取得する。
                let metadata = ctx.metadata();
                let symlink_metadata = ctx.symlink_metadata();
                let mut action_ctx = ActionContext {
                    path: ctx.path,
                    metadata,
                    symlink_metadata,
                    start_path: ctx.start_path,
                    depth: ctx.depth,
                    output_manager: &self.output_manager,
                    batch_executor: &mut self.batch_executor,
                    exit_code: &mut self.exit_code,
                };

                match action.execute(&mut action_ctx)? {
                    ActionResult::Continue => Ok((true, false, false)),
                    ActionResult::False => Ok((false, false, false)),
                    ActionResult::Prune => Ok((true, true, false)),
                    ActionResult::Quit => Ok((true, false, true)),
                }
            }

            Expression::Not(inner) => {
                let (result, prune, quit) = self.evaluate_expression(inner, ctx)?;
                Ok((!result, prune, quit))
            }

            Expression::And(left, right) => {
                let (left_result, left_prune, left_quit) = self.evaluate_expression(left, ctx)?;
                if left_quit {
                    return Ok((left_result, left_prune, true));
                }
                if !left_result {
                    return Ok((false, left_prune, false));
                }
                let (right_result, right_prune, right_quit) =
                    self.evaluate_expression(right, ctx)?;
                Ok((right_result, left_prune || right_prune, right_quit))
            }

            Expression::Or(left, right) => {
                let (left_result, left_prune, left_quit) = self.evaluate_expression(left, ctx)?;
                if left_quit {
                    return Ok((left_result, left_prune, true));
                }
                if left_result {
                    return Ok((true, left_prune, false));
                }
                let (right_result, right_prune, right_quit) =
                    self.evaluate_expression(right, ctx)?;
                Ok((right_result, left_prune || right_prune, right_quit))
            }

            Expression::List(left, right) => {
                let (_, left_prune, left_quit) = self.evaluate_expression(left, ctx)?;
                if left_quit {
                    return Ok((false, left_prune, true));
                }
                let (right_result, right_prune, right_quit) =
                    self.evaluate_expression(right, ctx)?;
                Ok((right_result, left_prune || right_prune, right_quit))
            }
        }
    }
}

pub(crate) fn should_follow_metadata(mode: FollowSymlinks, is_commandline_arg: bool) -> bool {
    match mode {
        FollowSymlinks::Always => true,
        FollowSymlinks::Commandline => is_commandline_arg,
        FollowSymlinks::Never => false,
    }
}

/// -daystart 用の基準時刻「今日の終わり（明日 00:00 ローカル時刻）」を返す
fn compute_day_end() -> Option<SystemTime> {
    use chrono::{Duration, Local};

    let tomorrow = Local::now().date_naive().checked_add_signed(Duration::days(1))?;
    let midnight = tomorrow.and_hms_opt(0, 0, 0)?;
    let local = midnight.and_local_timezone(Local).single()?;
    let timestamp = local.timestamp();
    if timestamp >= 0 {
        Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64))
    } else {
        None
    }
}
