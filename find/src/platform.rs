//! プラットフォーム固有の機能を抽象化するモジュール

use std::fs::Metadata;

// ============================================================================
// Unix実装
// ============================================================================
#[cfg(unix)]
mod platform_impl {
    use std::ffi::{CStr, CString};
    use std::fs::Metadata;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    pub fn get_uid(meta: &Metadata) -> u32 {
        meta.uid()
    }

    pub fn get_gid(meta: &Metadata) -> u32 {
        meta.gid()
    }

    pub fn get_mode(meta: &Metadata) -> u32 {
        meta.mode()
    }

    pub fn get_nlink(meta: &Metadata) -> u64 {
        meta.nlink()
    }

    pub fn get_ino(meta: &Metadata) -> u64 {
        meta.ino()
    }

    pub fn get_dev(meta: &Metadata) -> u64 {
        meta.dev()
    }

    pub fn get_ctime(meta: &Metadata) -> SystemTime {
        let ctime = meta.ctime();
        if ctime >= 0 {
            UNIX_EPOCH + Duration::from_secs(ctime as u64)
        } else {
            UNIX_EPOCH
        }
    }

    #[allow(dead_code)]
    pub fn get_blocks(meta: &Metadata) -> u64 {
        meta.blocks()
    }

    pub fn current_uid() -> u32 {
        unsafe { libc::getuid() }
    }

    pub fn current_gid() -> u32 {
        unsafe { libc::getgid() }
    }

    pub fn get_user_name(uid: u32) -> Option<String> {
        unsafe {
            let passwd = libc::getpwuid(uid);
            if passwd.is_null() {
                None
            } else {
                let name = CStr::from_ptr((*passwd).pw_name);
                Some(name.to_string_lossy().into_owned())
            }
        }
    }

    pub fn get_group_name(gid: u32) -> Option<String> {
        unsafe {
            let group = libc::getgrgid(gid);
            if group.is_null() {
                None
            } else {
                let name = CStr::from_ptr((*group).gr_name);
                Some(name.to_string_lossy().into_owned())
            }
        }
    }

    pub fn get_user_by_name(name: &str) -> Option<u32> {
        let c_name = CString::new(name).ok()?;
        unsafe {
            let passwd = libc::getpwnam(c_name.as_ptr());
            if passwd.is_null() {
                None
            } else {
                Some((*passwd).pw_uid)
            }
        }
    }

    pub fn get_group_by_name(name: &str) -> Option<u32> {
        let c_name = CString::new(name).ok()?;
        unsafe {
            let group = libc::getgrnam(c_name.as_ptr());
            if group.is_null() {
                None
            } else {
                Some((*group).gr_gid)
            }
        }
    }

    pub fn user_exists(uid: u32) -> bool {
        unsafe { !libc::getpwuid(uid).is_null() }
    }

    pub fn group_exists(gid: u32) -> bool {
        unsafe { !libc::getgrgid(gid).is_null() }
    }

    pub fn is_readable(meta: &Metadata) -> bool {
        let mode = meta.permissions().mode();
        let uid = current_uid();
        let gid = current_gid();

        if uid == 0 {
            true
        } else if meta.uid() == uid {
            mode & 0o400 != 0
        } else if meta.gid() == gid {
            mode & 0o040 != 0
        } else {
            mode & 0o004 != 0
        }
    }

    pub fn is_writable(meta: &Metadata) -> bool {
        let mode = meta.permissions().mode();
        let uid = current_uid();
        let gid = current_gid();

        if uid == 0 {
            true
        } else if meta.uid() == uid {
            mode & 0o200 != 0
        } else if meta.gid() == gid {
            mode & 0o020 != 0
        } else {
            mode & 0o002 != 0
        }
    }

    pub fn is_executable(meta: &Metadata) -> bool {
        let mode = meta.permissions().mode();
        let uid = current_uid();
        let gid = current_gid();

        if uid == 0 {
            mode & 0o111 != 0
        } else if meta.uid() == uid {
            mode & 0o100 != 0
        } else if meta.gid() == gid {
            mode & 0o010 != 0
        } else {
            mode & 0o001 != 0
        }
    }

    pub fn is_block_device(meta: &Metadata) -> bool {
        use std::os::unix::fs::FileTypeExt;
        meta.file_type().is_block_device()
    }

    pub fn is_char_device(meta: &Metadata) -> bool {
        use std::os::unix::fs::FileTypeExt;
        meta.file_type().is_char_device()
    }

    pub fn is_fifo(meta: &Metadata) -> bool {
        use std::os::unix::fs::FileTypeExt;
        meta.file_type().is_fifo()
    }

    pub fn is_socket(meta: &Metadata) -> bool {
        use std::os::unix::fs::FileTypeExt;
        meta.file_type().is_socket()
    }

    pub fn get_umask() -> u32 {
        unsafe {
            let current = libc::umask(0);
            libc::umask(current);
            current as u32
        }
    }

    pub fn format_mode_symbolic_internal(mode: u32, meta: &Metadata) -> String {
        use std::os::unix::fs::FileTypeExt;

        let file_type = meta.file_type();
        let ft = if file_type.is_block_device() {
            'b'
        } else if file_type.is_char_device() {
            'c'
        } else if file_type.is_fifo() {
            'p'
        } else if file_type.is_socket() {
            's'
        } else if meta.is_dir() {
            'd'
        } else if file_type.is_symlink() {
            'l'
        } else {
            '-'
        };

        format_mode_string(ft, mode)
    }

    fn format_mode_string(ft: char, mode: u32) -> String {
        let mut result = String::with_capacity(10);
        result.push(ft);

        // Owner
        result.push(if mode & 0o400 != 0 { 'r' } else { '-' });
        result.push(if mode & 0o200 != 0 { 'w' } else { '-' });
        result.push(if mode & 0o4000 != 0 {
            if mode & 0o100 != 0 {
                's'
            } else {
                'S'
            }
        } else if mode & 0o100 != 0 {
            'x'
        } else {
            '-'
        });

        // Group
        result.push(if mode & 0o040 != 0 { 'r' } else { '-' });
        result.push(if mode & 0o020 != 0 { 'w' } else { '-' });
        result.push(if mode & 0o2000 != 0 {
            if mode & 0o010 != 0 {
                's'
            } else {
                'S'
            }
        } else if mode & 0o010 != 0 {
            'x'
        } else {
            '-'
        });

        // Other
        result.push(if mode & 0o004 != 0 { 'r' } else { '-' });
        result.push(if mode & 0o002 != 0 { 'w' } else { '-' });
        result.push(if mode & 0o1000 != 0 {
            if mode & 0o001 != 0 {
                't'
            } else {
                'T'
            }
        } else if mode & 0o001 != 0 {
            'x'
        } else {
            '-'
        });

        result
    }
}

// ============================================================================
// Windows実装
// ============================================================================
#[cfg(windows)]
mod platform_impl {
    use std::fs::Metadata;
    use std::os::windows::fs::MetadataExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn get_uid(_meta: &Metadata) -> u32 {
        0
    }

    pub fn get_gid(_meta: &Metadata) -> u32 {
        0
    }

    pub fn get_mode(meta: &Metadata) -> u32 {
        let attrs = meta.file_attributes();
        let mut mode: u32 = 0o644;

        if meta.is_dir() {
            mode = 0o755;
        }

        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        if attrs & FILE_ATTRIBUTE_READONLY != 0 {
            mode &= !0o222;
        }

        mode
    }

    pub fn get_nlink(_meta: &Metadata) -> u64 {
        1
    }

    pub fn get_ino(_meta: &Metadata) -> u64 {
        0
    }

    pub fn get_dev(_meta: &Metadata) -> u64 {
        0
    }

    pub fn get_ctime(meta: &Metadata) -> SystemTime {
        meta.created().unwrap_or(UNIX_EPOCH)
    }

    #[allow(dead_code)]
    pub fn get_blocks(meta: &Metadata) -> u64 {
        (meta.len() + 511) / 512
    }

    #[allow(dead_code)]
    pub fn current_uid() -> u32 {
        0
    }

    #[allow(dead_code)]
    pub fn current_gid() -> u32 {
        0
    }

    pub fn get_user_name(_uid: u32) -> Option<String> {
        std::env::var("USERNAME").ok()
    }

    pub fn get_group_name(_gid: u32) -> Option<String> {
        Some("Users".to_string())
    }

    pub fn get_user_by_name(name: &str) -> Option<u32> {
        if let Ok(current) = std::env::var("USERNAME") {
            if current.eq_ignore_ascii_case(name) {
                return Some(0);
            }
        }
        None
    }

    pub fn get_group_by_name(name: &str) -> Option<u32> {
        if name.eq_ignore_ascii_case("Users") || name.eq_ignore_ascii_case("Administrators") {
            Some(0)
        } else {
            None
        }
    }

    pub fn user_exists(_uid: u32) -> bool {
        true
    }

    pub fn group_exists(_gid: u32) -> bool {
        true
    }

    pub fn is_readable(_meta: &Metadata) -> bool {
        true
    }

    pub fn is_writable(meta: &Metadata) -> bool {
        let attrs = meta.file_attributes();
        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        attrs & FILE_ATTRIBUTE_READONLY == 0
    }

    pub fn is_executable(meta: &Metadata) -> bool {
        meta.is_dir()
    }

    pub fn is_block_device(_meta: &Metadata) -> bool {
        false
    }

    pub fn is_char_device(_meta: &Metadata) -> bool {
        false
    }

    pub fn is_fifo(_meta: &Metadata) -> bool {
        false
    }

    pub fn is_socket(_meta: &Metadata) -> bool {
        false
    }

    pub fn get_umask() -> u32 {
        0
    }

    pub fn format_mode_symbolic_internal(mode: u32, meta: &Metadata) -> String {
        let ft = if meta.is_dir() {
            'd'
        } else if meta.file_type().is_symlink() {
            'l'
        } else {
            '-'
        };

        let mut result = String::with_capacity(10);
        result.push(ft);

        // Owner
        result.push(if mode & 0o400 != 0 { 'r' } else { '-' });
        result.push(if mode & 0o200 != 0 { 'w' } else { '-' });
        result.push(if mode & 0o4000 != 0 {
            if mode & 0o100 != 0 {
                's'
            } else {
                'S'
            }
        } else if mode & 0o100 != 0 {
            'x'
        } else {
            '-'
        });

        // Group
        result.push(if mode & 0o040 != 0 { 'r' } else { '-' });
        result.push(if mode & 0o020 != 0 { 'w' } else { '-' });
        result.push(if mode & 0o2000 != 0 {
            if mode & 0o010 != 0 {
                's'
            } else {
                'S'
            }
        } else if mode & 0o010 != 0 {
            'x'
        } else {
            '-'
        });

        // Other
        result.push(if mode & 0o004 != 0 { 'r' } else { '-' });
        result.push(if mode & 0o002 != 0 { 'w' } else { '-' });
        result.push(if mode & 0o1000 != 0 {
            if mode & 0o001 != 0 {
                't'
            } else {
                'T'
            }
        } else if mode & 0o001 != 0 {
            'x'
        } else {
            '-'
        });

        result
    }
}

// ============================================================================
// 共通インターフェース
// ============================================================================
pub use platform_impl::*;

/// ファイルタイプを判定
pub fn get_file_type_char(meta: &Metadata, is_symlink: bool) -> char {
    if is_symlink {
        'l'
    } else if meta.is_dir() {
        'd'
    } else if meta.is_file() {
        'f'
    } else if is_block_device(meta) {
        'b'
    } else if is_char_device(meta) {
        'c'
    } else if is_fifo(meta) {
        'p'
    } else if is_socket(meta) {
        's'
    } else {
        '?'
    }
}

/// パーミッションを記号形式で表示
pub fn format_mode_symbolic(mode: u32, meta: &Metadata) -> String {
    format_mode_symbolic_internal(mode, meta)
}

// ============================================================================
// Windows Job Object — 子プロセス管理
// ============================================================================
//
// Job Object に find プロセスを追加することで、find が終了した際に
// -exec で起動した子プロセスも自動的に終了する。
// これにより Ctrl-C 時の孤児プロセスを防ぐ。

#[cfg(windows)]
pub mod job {
    use std::io;

    /// Windows Job Object のラッパー。
    /// 作成時に「親ジョブ終了時に子を全て kill」フラグを設定する。
    pub struct JobObject(windows_sys::Win32::Foundation::HANDLE);

    impl JobObject {
        /// 新しい Job Object を作成し、自プロセスを追加する。
        /// 失敗しても panic せず `Err` を返す。
        pub fn create_and_assign_self() -> io::Result<Self> {
            use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
            use windows_sys::Win32::System::JobObjects::{
                AssignProcessToJobObject, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            };
            use windows_sys::Win32::System::Threading::GetCurrentProcess;

            // windows-sys 0.52 では CreateJobObjectW が削除されたため
            // Win32 API を直接 extern 宣言する。
            extern "system" {
                fn CreateJobObjectW(
                    lpjobattributes: *const std::ffi::c_void,
                    lpname: *const u16,
                ) -> windows_sys::Win32::Foundation::HANDLE;
            }

            unsafe {
                // Job Object 作成
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job == INVALID_HANDLE_VALUE || job == 0 {
                    return Err(io::Error::last_os_error());
                }

                // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE を設定
                // → Job Object の最後のハンドルが閉じた時（find 終了時）に全子プロセスを kill
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }

                // 自プロセスを Job Object に追加
                let proc = GetCurrentProcess();
                let ok = AssignProcessToJobObject(job, proc);
                if ok == 0 {
                    // すでに別の Job Object に属している場合は非致命的エラーとして無視
                    // (Visual Studio デバッガ等が既に Job Object を割り当てている場合がある)
                }

                Ok(JobObject(job))
            }
        }
    }

    impl Drop for JobObject {
        fn drop(&mut self) {
            use windows_sys::Win32::Foundation::CloseHandle;
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Windows 以外では no-op なスタブを提供する。
#[cfg(not(windows))]
pub mod job {
    pub struct JobObject;
    impl JobObject {
        #[inline]
        pub fn create_and_assign_self() -> std::io::Result<Self> {
            Ok(JobObject)
        }
    }
}
