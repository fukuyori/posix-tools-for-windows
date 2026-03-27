use std::collections::VecDeque;
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
}

impl Walker {
    pub fn new(options: GlobalOptions, expression: Option<Expression>) -> Self {
        let use_default_print = expression
            .as_ref()
            .map(|e| !Self::contains_non_default_action(e))
            .unwrap_or(true);

        Walker {
            options,
            expression,
            output_manager: OutputManager::new(),
            batch_executor: BatchExecutor::new(),
            use_default_print,
            exit_code: 0,
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
        let now = SystemTime::now();

        for start_path in start_paths {
            if !start_path.exists() {
                eprintln!(
                    "{}",
                    messages::err_path_not_found(&start_path.display().to_string())
                );
                self.exit_code = 1;
                continue;
            }

            let result: Result<(), String> = if self.options.depth_first {
                self.walk_depth_first(start_path, start_path, 0, now)
                    .map(|_| ())
            } else {
                self.walk_breadth_first(start_path, now)
            };

            if let Err(e) = result {
                eprintln!("{}", e);
                self.exit_code = 1;
            }
        }

        self.batch_executor.flush();
        self.output_manager.flush_all();
    }

    fn walk_breadth_first(&mut self, start: &Path, now: SystemTime) -> Result<(), String> {
        struct Entry {
            path: PathBuf,
            depth: usize,
        }

        let start_dev = fs::metadata(start).map(|m| platform::get_dev(&m)).ok();
        let mut queue: VecDeque<Entry> = VecDeque::new();
        queue.push_back(Entry {
            path: start.to_path_buf(),
            depth: 0,
        });

        while let Some(entry) = queue.pop_front() {
            if let Some(max) = self.options.max_depth {
                if entry.depth > max {
                    continue;
                }
            }

            let (metadata, symlink_metadata) =
                match self.get_metadata(&entry.path, entry.depth == 0) {
                    Some(m) => m,
                    None => {
                        self.exit_code = 1;
                        continue;
                    }
                };

            if self.options.xdev && start_dev.is_some() {
                if platform::get_dev(&metadata) != start_dev.unwrap() {
                    continue;
                }
            }

            // is_dir の判定はウォーカー自身が必要なので metadata を使う
            let is_dir = metadata.is_dir();

            let (_matched, should_prune, should_quit) = self.evaluate_and_execute(
                &entry.path,
                if symlink_metadata.is_some() {
                    symlink_metadata.as_ref().unwrap().clone()
                } else {
                    metadata.clone()
                },
                if symlink_metadata.is_some() {
                    Some(metadata)
                } else {
                    None
                },
                start,
                entry.depth,
                now,
            )?;

            if should_quit {
                return Ok(());
            }

            if is_dir && !should_prune {
                let should_descend = match self.options.max_depth {
                    Some(max) => entry.depth < max,
                    None => true,
                };

                if should_descend {
                    match fs::read_dir(&entry.path) {
                        Ok(read_dir) => {
                            for dir_entry in read_dir.flatten() {
                                queue.push_back(Entry {
                                    path: dir_entry.path(),
                                    depth: entry.depth + 1,
                                });
                            }
                        }
                        Err(_e) => {
                            eprintln!(
                                "{}",
                                messages::warn_cannot_read_dir(&entry.path.display().to_string())
                            );
                            self.exit_code = 1;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn walk_depth_first(
        &mut self,
        path: &Path,
        start: &Path,
        depth: usize,
        now: SystemTime,
    ) -> Result<bool, String> {
        if let Some(max) = self.options.max_depth {
            if depth > max {
                return Ok(false);
            }
        }

        let (metadata, symlink_metadata) = match self.get_metadata(path, depth == 0) {
            Some(m) => m,
            None => {
                self.exit_code = 1;
                return Ok(false);
            }
        };

        let start_dev = fs::metadata(start).map(|m| platform::get_dev(&m)).ok();
        if self.options.xdev && start_dev.is_some() {
            if platform::get_dev(&metadata) != start_dev.unwrap() {
                return Ok(false);
            }
        }

        let is_dir = metadata.is_dir();

        if is_dir {
            let should_descend = match self.options.max_depth {
                Some(max) => depth < max,
                None => true,
            };

            if should_descend {
                match fs::read_dir(path) {
                    Ok(read_dir) => {
                        for dir_entry in read_dir.flatten() {
                            let quit =
                                self.walk_depth_first(&dir_entry.path(), start, depth + 1, now)?;
                            if quit {
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
        }

        let (_, _, should_quit) = self.evaluate_and_execute(
            path,
            if symlink_metadata.is_some() {
                symlink_metadata.as_ref().unwrap().clone()
            } else {
                metadata.clone()
            },
            if symlink_metadata.is_some() {
                Some(metadata)
            } else {
                None
            },
            start,
            depth,
            now,
        )?;

        Ok(should_quit)
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
        symlink_meta: Metadata,
        followed_meta: Option<Metadata>,
        start: &Path,
        depth: usize,
        now: SystemTime,
    ) -> Result<(bool, bool, bool), String> {
        if let Some(min) = self.options.min_depth {
            if depth < min {
                return Ok((false, false, false));
            }
        }

        let follow_symlinks = matches!(self.options.follow_symlinks, FollowSymlinks::Always);
        let eval_ctx = EvalContext::new(
            path,
            start,
            depth,
            now,
            symlink_meta,
            followed_meta,
            follow_symlinks,
        );

        // Clone the expression to avoid borrowing issues
        let expr_opt = self.expression.clone();
        let (matched, should_prune, should_quit) = if let Some(ref expr) = expr_opt {
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
