use std::cell::OnceCell;
use std::fs::{self, Metadata};
use std::path::Path;
use std::time::{Duration, SystemTime};

use glob::Pattern;
use regex::Regex;

use crate::platform;

/// ファイルタイプ
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    BlockDevice,  // b
    CharDevice,   // c
    Directory,    // d
    RegularFile,  // f
    SymbolicLink, // l
    Pipe,         // p
    Socket,       // s
}

impl FileType {
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'b' => Some(FileType::BlockDevice),
            'c' => Some(FileType::CharDevice),
            'd' => Some(FileType::Directory),
            'f' => Some(FileType::RegularFile),
            'l' => Some(FileType::SymbolicLink),
            'p' => Some(FileType::Pipe),
            's' => Some(FileType::Socket),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn to_char(self) -> char {
        match self {
            FileType::BlockDevice => 'b',
            FileType::CharDevice => 'c',
            FileType::Directory => 'd',
            FileType::RegularFile => 'f',
            FileType::SymbolicLink => 'l',
            FileType::Pipe => 'p',
            FileType::Socket => 's',
        }
    }

    pub fn matches(&self, meta: &Metadata, is_symlink: bool) -> bool {
        match self {
            FileType::BlockDevice => platform::is_block_device(meta),
            FileType::CharDevice => platform::is_char_device(meta),
            FileType::Directory => meta.is_dir(),
            FileType::RegularFile => meta.is_file(),
            FileType::SymbolicLink => is_symlink,
            FileType::Pipe => platform::is_fifo(meta),
            FileType::Socket => platform::is_socket(meta),
        }
    }
}

/// 数値比較の種類
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericComparison {
    Exactly(i64),     // n
    GreaterThan(i64), // +n
    LessThan(i64),    // -n
}

impl NumericComparison {
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }

        let (sign, num_str) = if s.starts_with('+') {
            ('+', &s[1..])
        } else if s.starts_with('-') {
            ('-', &s[1..])
        } else {
            ('=', s)
        };

        let num: i64 = num_str.parse().ok()?;

        Some(match sign {
            '+' => NumericComparison::GreaterThan(num),
            '-' => NumericComparison::LessThan(num),
            _ => NumericComparison::Exactly(num),
        })
    }

    pub fn matches(&self, value: i64) -> bool {
        match self {
            NumericComparison::Exactly(n) => value == *n,
            NumericComparison::GreaterThan(n) => value > *n,
            NumericComparison::LessThan(n) => value < *n,
        }
    }
}

/// サイズ単位
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeUnit {
    Bytes,     // c
    Words,     // w (2 bytes)
    Blocks512, // b (512 bytes, default)
    Kibi,      // k (1024)
    Mebi,      // M (1024^2)
    Gibi,      // G (1024^3)
}

impl SizeUnit {
    pub fn multiplier(&self) -> u64 {
        match self {
            SizeUnit::Bytes => 1,
            SizeUnit::Words => 2,
            SizeUnit::Blocks512 => 512,
            SizeUnit::Kibi => 1024,
            SizeUnit::Mebi => 1024 * 1024,
            SizeUnit::Gibi => 1024 * 1024 * 1024,
        }
    }
}

/// サイズ比較
#[derive(Debug, Clone)]
pub struct SizeComparison {
    pub comparison: NumericComparison,
    pub unit: SizeUnit,
}

impl SizeComparison {
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }

        let (num_part, unit) = if let Some(c) = s.chars().last() {
            match c {
                'c' => (&s[..s.len() - 1], SizeUnit::Bytes),
                'w' => (&s[..s.len() - 1], SizeUnit::Words),
                'b' => (&s[..s.len() - 1], SizeUnit::Blocks512),
                'k' => (&s[..s.len() - 1], SizeUnit::Kibi),
                'M' => (&s[..s.len() - 1], SizeUnit::Mebi),
                'G' => (&s[..s.len() - 1], SizeUnit::Gibi),
                _ => (s, SizeUnit::Blocks512),
            }
        } else {
            return None;
        };

        let comparison = NumericComparison::parse(num_part)?;
        Some(SizeComparison { comparison, unit })
    }

    pub fn matches(&self, size: u64) -> bool {
        let unit_size = self.unit.multiplier();
        let units = match self.unit {
            SizeUnit::Bytes => size as i64,
            _ => ((size + unit_size - 1) / unit_size) as i64,
        };
        self.comparison.matches(units)
    }
}

/// パーミッション比較モード
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PermMode {
    Exact(u32), // mode - exactly
    All(u32),   // -mode - all bits set
    Any(u32),   // /mode - any bit set
}

impl PermMode {
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }

        let (mode_type, mode_str) = if s.starts_with('-') {
            ('a', &s[1..])
        } else if s.starts_with('/') {
            ('y', &s[1..])
        } else {
            ('e', s)
        };

        let mode = Self::parse_mode(mode_str)?;

        Some(match mode_type {
            'a' => PermMode::All(mode),
            'y' => PermMode::Any(mode),
            _ => PermMode::Exact(mode),
        })
    }

    fn parse_mode(s: &str) -> Option<u32> {
        // Try octal first
        if let Ok(mode) = u32::from_str_radix(s, 8) {
            return Some(mode & 0o7777);
        }

        Self::parse_symbolic_mode(s)
    }

    fn parse_symbolic_mode(s: &str) -> Option<u32> {
        let mut mode: u32 = 0;
        for part in s.split(',') {
            mode = Self::apply_symbolic_part(mode, part)?;
        }
        Some(mode & 0o7777)
    }

    fn apply_symbolic_part(mode: u32, s: &str) -> Option<u32> {
        let mut chars = s.chars().peekable();
        let mut who_specified = false;
        let mut who: u8 = 0;

        // Parse who (u, g, o, a)
        while let Some(&c) = chars.peek() {
            match c {
                'u' => {
                    who |= 0b001;
                    who_specified = true;
                    chars.next();
                }
                'g' => {
                    who |= 0b010;
                    who_specified = true;
                    chars.next();
                }
                'o' => {
                    who |= 0b100;
                    who_specified = true;
                    chars.next();
                }
                'a' => {
                    who |= 0b111;
                    who_specified = true;
                    chars.next();
                }
                _ => break,
            }
        }

        if !who_specified {
            who = 0b111; // default to all
        }

        // Parse operator
        let op = chars.next()?;
        if !matches!(op, '+' | '-' | '=') {
            return None;
        }

        // Parse permissions
        let mut perm_bits: u32 = 0;
        let apply_umask = !who_specified && op != '=';
        let umask = if apply_umask {
            platform::get_umask() & 0o777
        } else {
            0
        };
        for c in chars {
            match c {
                'r' => perm_bits |= Self::apply_umask(Self::permission_bits(who, 0o4, 0), umask),
                'w' => perm_bits |= Self::apply_umask(Self::permission_bits(who, 0o2, 0), umask),
                'x' => perm_bits |= Self::apply_umask(Self::permission_bits(who, 0o1, 0), umask),
                'X' => {
                    if mode & 0o111 != 0 {
                        perm_bits |= Self::apply_umask(Self::permission_bits(who, 0o1, 0), umask);
                    }
                }
                's' => {
                    if who & 0b001 != 0 {
                        perm_bits |= 0o4000;
                    }
                    if who & 0b010 != 0 {
                        perm_bits |= 0o2000;
                    }
                }
                't' => perm_bits |= 0o1000,
                'u' => perm_bits |= Self::copy_bits(mode, who, 0b001),
                'g' => perm_bits |= Self::copy_bits(mode, who, 0b010),
                'o' => perm_bits |= Self::copy_bits(mode, who, 0b100),
                _ => return None,
            }
        }

        let clear_mask = Self::who_clear_mask(who);
        Some(match op {
            '+' => mode | perm_bits,
            '-' => mode & !perm_bits,
            '=' => (mode & !clear_mask) | perm_bits,
            _ => return None,
        })
    }

    fn permission_bits(who: u8, perm: u32, shift_base: u32) -> u32 {
        let mut bits = 0;
        if who & 0b001 != 0 {
            bits |= perm << (6 + shift_base);
        }
        if who & 0b010 != 0 {
            bits |= perm << (3 + shift_base);
        }
        if who & 0b100 != 0 {
            bits |= perm << shift_base;
        }
        bits
    }

    fn copy_bits(mode: u32, target_who: u8, source_who: u8) -> u32 {
        let source = if source_who & 0b001 != 0 {
            (mode >> 6) & 0o7
        } else if source_who & 0b010 != 0 {
            (mode >> 3) & 0o7
        } else {
            mode & 0o7
        };

        let mut bits = 0;
        if target_who & 0b001 != 0 {
            bits |= source << 6;
        }
        if target_who & 0b010 != 0 {
            bits |= source << 3;
        }
        if target_who & 0b100 != 0 {
            bits |= source;
        }
        bits
    }

    fn who_clear_mask(who: u8) -> u32 {
        let mut mask = 0;
        if who & 0b001 != 0 {
            mask |= 0o4700;
        }
        if who & 0b010 != 0 {
            mask |= 0o2070;
        }
        if who & 0b100 != 0 {
            mask |= 0o1007;
        }
        mask
    }

    fn apply_umask(bits: u32, umask: u32) -> u32 {
        bits & !umask
    }

    pub fn matches(&self, file_mode: u32) -> bool {
        let file_perm = file_mode & 0o7777;
        match self {
            PermMode::Exact(mode) => file_perm == *mode,
            PermMode::All(mode) => (file_perm & mode) == *mode,
            PermMode::Any(mode) => *mode == 0 || (file_perm & mode) != 0,
        }
    }
}

/// -exec のタイプ
#[derive(Debug, Clone, PartialEq)]
pub enum ExecType {
    Each,  // {} \;
    Batch, // {} +
}

/// アクション
#[derive(Debug, Clone)]
pub enum Action {
    Print,
    Print0,
    FPrint(String),
    FPrint0(String),
    Printf(String),
    FPrintf(String, String), // (ファイル, フォーマット)
    Ls,
    FLs(String),
    Exec {
        command: Vec<String>,
        exec_type: ExecType,
        in_dir: bool,
    },
    Ok {
        command: Vec<String>,
        in_dir: bool,
    },
    Delete,
    Prune,
    Quit,
}

/// 時間のタイプ
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeType {
    Access, // atime
    Change, // ctime (status change)
    Modify, // mtime
    Birth,  // btime (creation time)
}

/// テスト式
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Test {
    // 名前関連
    Name {
        pattern: Pattern,
        case_insensitive: bool,
    },
    Path {
        pattern: Pattern,
        case_insensitive: bool,
    },
    Regex {
        regex: Regex,
        case_insensitive: bool,
    },
    /// -lname / -ilname: シンボリックリンクの内容がパターンにマッチ
    Lname {
        pattern: Pattern,
        case_insensitive: bool,
    },
    /// -fstype: ファイルシステムのタイプ名
    Fstype(String),

    // タイプ
    Type(FileType),
    Xtype(FileType),

    // サイズ
    Size(SizeComparison),
    Empty,

    // 時間
    Time {
        time_type: TimeType,
        comparison: NumericComparison,
        minutes: bool,
        /// -daystart が先行して指定されていた場合、今日の始まりを基準に測る
        daystart: bool,
    },
    /// -used n: 最終アクセスがステータス変更から n 日後
    Used(NumericComparison),
    Newer {
        reference_time: SystemTime,
    },
    NewerXY {
        x: TimeType,
        y: TimeType,
        reference: SystemTime,
    },

    // 所有者
    User(u32),
    Group(u32),
    Uid(NumericComparison),
    Gid(NumericComparison),
    NoUser,
    NoGroup,

    // パーミッション
    Perm(PermMode),
    Readable,
    Writable,
    Executable,

    // その他
    Links(NumericComparison),
    Inum(NumericComparison),
    Samefile {
        dev: u64,
        ino: u64,
    },

    // 定数
    True,
    False,
}

/// 式ノード
#[derive(Debug, Clone)]
pub enum Expression {
    Test(Test),
    Action(Action),
    Not(Box<Expression>),
    And(Box<Expression>, Box<Expression>),
    Or(Box<Expression>, Box<Expression>),
    List(Box<Expression>, Box<Expression>),
}

/// 評価コンテキスト
/// 式評価コンテキスト。
///
/// `metadata` と `symlink_metadata` は遅延取得（`OnceCell`）。
/// `-name` / `-path` / `-depth` など「ファイル名とパスだけで判定できる式」では
/// `fs::metadata()` の呼び出しをスキップできるため、特に Windows で効果が大きい。
pub struct EvalContext<'a> {
    pub path: &'a Path,
    pub start_path: &'a Path,
    pub depth: usize,
    pub now: SystemTime,
    /// -daystart 用の基準時刻（今日の終わり = 明日 00:00）。
    /// 未設定の場合は `now` と同じ。
    pub day_end: SystemTime,

    /// `symlink_metadata` の結果。`None` はシンボリックリンクではないことを示す
    /// （リンクかどうかは `symlink_meta_result` で判定する）。
    symlink_meta_result: Option<Metadata>,

    /// `metadata()` （リンク追跡後）の遅延結果。
    /// `symlink_meta_result` 自体が Some(&m) の場合はリンク先を指す。
    /// `follow_symlinks` が false の場合は `symlink_metadata` と同じ値を返す。
    lazy_meta: OnceCell<Option<Metadata>>,

    /// ウォーカーが `-L` などでシンボリックリンクを追跡するかどうか
    follow_symlinks: bool,
}

impl<'a> EvalContext<'a> {
    /// ウォーカーから呼ばれるコンストラクタ。
    /// `symlink_meta` は `fs::symlink_metadata()` の結果、
    /// `followed_meta` は リンク追跡後の `fs::metadata()` の結果（リンクでなければ同じ値）。
    pub fn new(
        path: &'a Path,
        start_path: &'a Path,
        depth: usize,
        now: SystemTime,
        symlink_meta: Metadata,
        followed_meta: Option<Metadata>,
        follow_symlinks: bool,
    ) -> Self {
        let is_symlink = symlink_meta.file_type().is_symlink();
        // symlink_meta_result: symlink の場合のみ Some で保持する
        let symlink_meta_result = if is_symlink {
            Some(symlink_meta.clone())
        } else {
            None
        };
        // lazy_meta: symlink を追跡した結果。非 symlink の場合は symlink_meta と同じ。
        let resolved = followed_meta.unwrap_or(symlink_meta);
        let lazy_meta = OnceCell::new();
        // 解決済みの metadata を事前に格納しておく
        let _ = lazy_meta.set(Some(resolved));

        EvalContext {
            path,
            start_path,
            depth,
            now,
            day_end: now,
            symlink_meta_result,
            lazy_meta,
            follow_symlinks,
        }
    }

    /// 通常の metadata（リンク追跡後）を返す。
    /// 未取得であれば `fs::metadata()` を呼んで結果をキャッシュする。
    pub fn metadata(&self) -> Option<&Metadata> {
        self.lazy_meta
            .get_or_init(|| {
                if self.follow_symlinks {
                    fs::metadata(self.path).ok()
                } else {
                    fs::symlink_metadata(self.path).ok()
                }
            })
            .as_ref()
    }

    /// `symlink_metadata`（リンク自体の情報）を返す。
    /// シンボリックリンクでない場合は `None`。
    pub fn symlink_metadata(&self) -> Option<&Metadata> {
        self.symlink_meta_result.as_ref()
    }

    /// ファイルがシンボリックリンクかどうか。
    #[inline]
    pub fn is_symlink(&self) -> bool {
        self.symlink_meta_result.is_some()
    }

    /// ウォーカーがシンボリックリンクを追跡するモードかどうか。
    #[inline]
    pub fn follows_symlinks(&self) -> bool {
        self.follow_symlinks
    }
}

impl Test {
    pub fn evaluate(&self, ctx: &EvalContext) -> bool {
        match self {
            Test::Name {
                pattern,
                case_insensitive,
            } => {
                if let Some(name) = ctx.path.file_name().and_then(|n| n.to_str()) {
                    let name = if *case_insensitive {
                        name.to_lowercase()
                    } else {
                        name.to_string()
                    };
                    pattern.matches(&name)
                } else {
                    false
                }
            }

            Test::Path {
                pattern,
                case_insensitive,
            } => {
                if let Some(path_str) = ctx.path.to_str() {
                    let path_str = normalize_glob_path(path_str);
                    let path_str = if *case_insensitive {
                        path_str.to_lowercase()
                    } else {
                        path_str
                    };
                    pattern.matches(&path_str)
                } else {
                    false
                }
            }

            Test::Regex {
                regex,
                case_insensitive: _,
            } => {
                if let Some(path_str) = ctx.path.to_str() {
                    regex.is_match(path_str)
                } else {
                    false
                }
            }

            // -type / -xtype はファイルタイプのみ参照。
            // symlink かどうかは EvalContext が持つ情報で判定できる。
            Test::Type(ft) => {
                let is_symlink = ctx.is_symlink();
                // metadata() は type 判定に必要（dir か file か）
                match ctx.metadata() {
                    Some(m) => ft.matches(m, is_symlink),
                    None => false,
                }
            }

            Test::Xtype(ft) => {
                // -xtype: -type の逆のリンク解決を行う（GNU 互換）。
                // * 非シンボリックリンク: -type と同じ
                // * シンボリックリンク（-P/-H で未追跡）: リンク先のタイプで判定。
                //   リンク切れは 'l' として扱う
                // * シンボリックリンク（-L で追跡済み）: 'l' のみ真
                if !ctx.is_symlink() {
                    return match ctx.metadata() {
                        Some(m) => ft.matches(m, false),
                        None => false,
                    };
                }
                if ctx.follows_symlinks() {
                    *ft == FileType::SymbolicLink
                } else {
                    match fs::metadata(ctx.path) {
                        Ok(m) => ft.matches(&m, false),
                        Err(_) => *ft == FileType::SymbolicLink,
                    }
                }
            }

            Test::Lname {
                pattern,
                case_insensitive,
            } => {
                if !ctx.is_symlink() {
                    return false;
                }
                match fs::read_link(ctx.path) {
                    Ok(target) => {
                        let target_str = target.to_string_lossy();
                        if *case_insensitive {
                            pattern.matches(&target_str.to_lowercase())
                        } else {
                            pattern.matches(&target_str)
                        }
                    }
                    Err(_) => false,
                }
            }

            Test::Fstype(name) => platform::get_fstype(ctx.path)
                .map(|t| t.eq_ignore_ascii_case(name))
                .unwrap_or(false),

            Test::Size(size_comp) => ctx
                .metadata()
                .map(|m| size_comp.matches(m.len()))
                .unwrap_or(false),

            Test::Empty => match ctx.metadata() {
                Some(m) if m.is_dir() => fs::read_dir(ctx.path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false),
                Some(m) if m.is_file() => m.len() == 0,
                _ => false,
            },

            Test::Time {
                time_type,
                comparison,
                minutes,
                daystart,
            } => {
                let Some(m) = ctx.metadata() else {
                    return false;
                };
                let file_time = match time_type {
                    TimeType::Access => m.accessed().ok(),
                    TimeType::Change => Some(platform::get_ctime(m)),
                    TimeType::Modify => m.modified().ok(),
                    TimeType::Birth => platform::get_btime(m),
                };
                if let Some(ft) = file_time {
                    // -daystart 指定時は「今日の終わり（明日 00:00）」を基準に測る
                    let base = if *daystart { ctx.day_end } else { ctx.now };
                    let duration = base.duration_since(ft).unwrap_or(Duration::ZERO);
                    let units = if *minutes {
                        (duration.as_secs() / 60) as i64
                    } else {
                        (duration.as_secs() / 86400) as i64
                    };
                    comparison.matches(units)
                } else {
                    false
                }
            }

            Test::Used(comparison) => {
                let Some(m) = ctx.metadata() else {
                    return false;
                };
                let Ok(atime) = m.accessed() else {
                    return false;
                };
                let ctime = platform::get_ctime(m);
                let days = atime
                    .duration_since(ctime)
                    .map(|d| (d.as_secs() / 86400) as i64)
                    .unwrap_or(0);
                comparison.matches(days)
            }

            Test::Newer { reference_time } => ctx
                .metadata()
                .and_then(|m| m.modified().ok())
                .map(|mt| mt > *reference_time)
                .unwrap_or(false),

            Test::NewerXY { x, y: _, reference } => {
                let Some(m) = ctx.metadata() else {
                    return false;
                };
                let file_time = match x {
                    TimeType::Access => m.accessed().ok(),
                    TimeType::Change => Some(platform::get_ctime(m)),
                    TimeType::Modify => m.modified().ok(),
                    TimeType::Birth => platform::get_btime(m),
                };
                file_time.map(|ft| ft > *reference).unwrap_or(false)
            }

            Test::User(uid) => ctx
                .metadata()
                .map(|m| platform::get_uid(m) == *uid)
                .unwrap_or(false),
            Test::Group(gid) => ctx
                .metadata()
                .map(|m| platform::get_gid(m) == *gid)
                .unwrap_or(false),
            Test::Uid(comp) => ctx
                .metadata()
                .map(|m| comp.matches(platform::get_uid(m) as i64))
                .unwrap_or(false),
            Test::Gid(comp) => ctx
                .metadata()
                .map(|m| comp.matches(platform::get_gid(m) as i64))
                .unwrap_or(false),

            Test::NoUser => ctx
                .metadata()
                .map(|m| !platform::user_exists(platform::get_uid(m)))
                .unwrap_or(false),
            Test::NoGroup => ctx
                .metadata()
                .map(|m| !platform::group_exists(platform::get_gid(m)))
                .unwrap_or(false),

            Test::Perm(perm) => ctx
                .metadata()
                .map(|m| perm.matches(platform::get_mode(m)))
                .unwrap_or(false),

            Test::Readable => ctx
                .metadata()
                .map(|m| platform::is_readable(m))
                .unwrap_or(false),
            Test::Writable => ctx
                .metadata()
                .map(|m| platform::is_writable(m))
                .unwrap_or(false),
            Test::Executable => ctx
                .metadata()
                .map(|m| platform::is_executable(m))
                .unwrap_or(false),

            Test::Links(comp) => file_ids(ctx)
                .map(|(_, _, nlink)| comp.matches(nlink as i64))
                .unwrap_or(false),
            Test::Inum(comp) => file_ids(ctx)
                .map(|(_, ino, _)| comp.matches(ino as i64))
                .unwrap_or(false),

            Test::Samefile { dev, ino } => file_ids(ctx)
                .map(|(d, i, _)| d == *dev && i == *ino)
                .unwrap_or(false),

            Test::True => true,
            Test::False => false,
        }
    }
}

/// (デバイス番号, inode 番号, ハードリンク数) を返す。
/// Unix は取得済みメタデータから、Windows はハンドル問い合わせで取得する。
fn file_ids(ctx: &EvalContext) -> Option<(u64, u64, u64)> {
    #[cfg(windows)]
    {
        platform::get_file_ids(ctx.path, ctx.follows_symlinks())
    }

    #[cfg(not(windows))]
    {
        ctx.metadata().map(|m| {
            (
                platform::get_dev(m),
                platform::get_ino(m),
                platform::get_nlink(m),
            )
        })
    }
}

fn normalize_glob_path(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('\\', "/")
    }

    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

impl std::fmt::Display for Expression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expression::Test(t) => write!(f, "{:?}", t),
            Expression::Action(a) => write!(f, "{:?}", a),
            Expression::Not(e) => write!(f, "NOT({})", e),
            Expression::And(a, b) => write!(f, "({} AND {})", a, b),
            Expression::Or(a, b) => write!(f, "({} OR {})", a, b),
            Expression::List(a, b) => write!(f, "({} , {})", a, b),
        }
    }
}
