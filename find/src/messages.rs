/// 日本語メッセージ定義

pub const HELP_MESSAGE: &str = r#"
使用法: find [パス...] [式]

Windows 指向の find コマンド（Rust 実装、POSIX 互換）

パス:
  検索を開始するディレクトリ。省略時はカレントディレクトリ (.)
  Windows では `src\*` のような開始パス glob も find 自身が展開

オプション（グローバル）:
  -H                  コマンドライン引数のシンボリックリンクを辿る
  -L                  全てのシンボリックリンクを辿る
  -P                  シンボリックリンクを辿らない（デフォルト）
  -maxdepth <レベル>  最大検索深度を指定
  -mindepth <レベル>  最小検索深度を指定
  -depth              深さ優先で処理（ディレクトリの前に内容を処理）
  -xdev               他のファイルシステムに入らない
  -mount              -xdev と同じ

テスト（条件式）:
  -name <パターン>    ファイル名がパターンにマッチ（glob形式）
  -iname <パターン>   大文字小文字を区別せずにファイル名がマッチ
  -path <パターン>    パス全体がパターンにマッチ
  -ipath <パターン>   大文字小文字を区別せずにパスがマッチ
  -regex <正規表現>   パスが正規表現にマッチ
  -iregex <正規表現>  大文字小文字を区別せず正規表現にマッチ

  -type <タイプ>      ファイルタイプで検索
                        b: ブロックデバイス
                        c: キャラクタデバイス
                        d: ディレクトリ
                        f: 通常ファイル
                        l: シンボリックリンク
                        p: 名前付きパイプ (FIFO)
                        s: ソケット

  -size <n>[cwbkMG]   ファイルサイズで検索
                        c: バイト
                        w: 2バイトワード
                        b: 512バイトブロック（デフォルト）
                        k: キビバイト (1024)
                        M: メビバイト (1024^2)
                        G: ギビバイト (1024^3)
                      +n: nより大きい, -n: nより小さい, n: ちょうどn

  -empty              空のファイルまたはディレクトリ
  -newer <ファイル>   指定ファイルより新しい
  -newerXY <参照>     タイムスタンプ比較（X,Y: a=atime, c=ctime, m=mtime, t=絶対時間）

  -atime <n>          アクセス時刻（日単位）
  -ctime <n>          ステータス変更時刻（日単位）
  -mtime <n>          修正時刻（日単位）
  -amin <n>           アクセス時刻（分単位）
  -cmin <n>           ステータス変更時刻（分単位）
  -mmin <n>           修正時刻（分単位）

  -user <名前/ID>     所有者で検索
  -group <名前/ID>    グループで検索
  -uid <ID>           ユーザーIDで検索
  -gid <ID>           グループIDで検索
  -nouser             所有者が存在しない
  -nogroup            グループが存在しない

  -perm <モード>      パーミッションで検索
                        モード: 8進数 (644) または記号形式 (u=rw,g=r,o=r)
                        -モード: 全ビットが設定されている
                        /モード: いずれかのビットが設定されている

  -readable           読み取り可能
  -writable           書き込み可能
  -executable         実行可能

  -links <n>          ハードリンク数で検索
  -inum <n>           inode番号で検索
  -samefile <ファイル> 同じinodeを持つファイル

  -true               常に真
  -false              常に偽

アクション:
  -print              パスを表示（デフォルト）
  -print0             NULLで区切って表示
  -fprint <ファイル>  ファイルに出力
  -fprint0 <ファイル> NULLで区切ってファイルに出力

  -printf <フォーマット>  フォーマット指定で出力
    ディレクティブ:
      %%  リテラル %
      %p  ファイル名（パス）
      %f  ベース名
      %h  ディレクトリ名
      %P  開始点からの相対パス
      %H  開始点
      %d  深度
      %s  サイズ（バイト）
      %k  サイズ（キビバイト）
      %b  512バイトブロック数
      %m  パーミッション（8進数）
      %M  パーミッション（記号形式）
      %u  ユーザー名
      %U  ユーザーID
      %g  グループ名
      %G  グループID
      %l  シンボリックリンク先
      %i  inode番号
      %n  ハードリンク数
      %y  タイプ（1文字）
      %Y  タイプ（リンク先を辿る）
      %a  アクセス時刻
      %A<フォーマット>  アクセス時刻（strftime形式）
      %c  ステータス変更時刻
      %C<フォーマット>  ステータス変更時刻（strftime形式）
      %t  修正時刻
      %T<フォーマット>  修正時刻（strftime形式）
      \n  改行
      \t  タブ
      \0  NULL

  -ls                 ls -dils 形式で表示
  -fls <ファイル>     ls形式でファイルに出力

  -exec <コマンド> {} \;       各ファイルでコマンドを実行
  -exec <コマンド> {} +        まとめてコマンドを実行
  -execdir <コマンド> {} \;    ファイルのディレクトリでコマンドを実行
  -execdir <コマンド> {} +     ファイルのディレクトリでまとめて実行
  -ok <コマンド> {} \;         確認してからコマンドを実行
  -okdir <コマンド> {} \;      確認してからディレクトリで実行

  -delete             ファイルを削除（暗黙的に -depth を有効化）
  -prune              ディレクトリに入らない
  -quit               即座に終了

論理演算子:
  ( <式> )            グループ化
  ! <式>, -not <式>   否定
  \(...\), \!, \;     Windows/移植用のエスケープ表記も利用可能
  <式1> -a <式2>      論理積（AND、デフォルト）
  <式1> -and <式2>    論理積
  <式1> -o <式2>      論理和（OR）
  <式1> -or <式2>     論理和
  <式1> , <式2>       リスト（両方評価、式2の結果を返す）

例:
  find .                           カレント以下の全ファイルを表示
  find .\src\* -name "*.rs"        Windows で開始パス glob を使う
  find . "\!" -name "*.log"        PowerShell で否定を使う
  find /home -name "*.txt"         .txtファイルを検索
  find . -type f -size +1M         1MBより大きいファイル
  find . -name "*.log" -delete     .logファイルを削除
  find . -type f -exec chmod 644 {} \;   全ファイルのパーミッションを変更
  find . -mtime -7                 7日以内に変更されたファイル
  find . -empty -type d            空のディレクトリ
  find . \( -name "*.c" -o -name "*.h" \)  .cまたは.hファイル
  find . "(" -name "*.c" -o -name "*.h" ")"   PowerShell 向け引用符版

終了コード:
  0  全て成功
  1  エラー発生

バージョン: 1.0.0
"#;

// エラーメッセージ
pub fn err_missing_argument(opt: &str) -> String {
    format!("エラー: '{}' には引数が必要です", opt)
}

pub fn err_invalid_argument(opt: &str, arg: &str) -> String {
    format!("エラー: '{}' の引数 '{}' が不正です", opt, arg)
}

pub fn err_unknown_option(opt: &str) -> String {
    format!("エラー: 不明なオプション '{}'", opt)
}

pub fn err_path_not_found(path: &str) -> String {
    format!("エラー: パス '{}' が見つかりません", path)
}

#[allow(dead_code)]
pub fn err_permission_denied(path: &str) -> String {
    format!("エラー: '{}' へのアクセスが拒否されました", path)
}

pub fn err_invalid_type(t: &str) -> String {
    format!(
        "エラー: 不正なファイルタイプ '{}' (有効: b, c, d, f, l, p, s)",
        t
    )
}

pub fn err_invalid_size(s: &str) -> String {
    format!("エラー: 不正なサイズ指定 '{}' (例: +1M, -100k, 512c)", s)
}

pub fn err_invalid_time(s: &str) -> String {
    format!("エラー: 不正な時間指定 '{}' (例: +7, -1, 0)", s)
}

pub fn err_invalid_perm(s: &str) -> String {
    format!("エラー: 不正なパーミッション '{}' (例: 644, -755, /u+x)", s)
}

pub fn err_invalid_regex(s: &str, err: &str) -> String {
    format!("エラー: 不正な正規表現 '{}': {}", s, err)
}

pub fn err_missing_exec_terminator() -> String {
    "エラー: -exec/-ok には ';' または '+' が必要です".to_string()
}

pub fn err_unmatched_paren() -> String {
    "エラー: 括弧が対応していません".to_string()
}

pub fn err_exec_failed(cmd: &str, err: &str) -> String {
    format!("エラー: コマンド '{}' の実行に失敗しました: {}", cmd, err)
}

pub fn err_delete_failed(path: &str, err: &str) -> String {
    format!("エラー: '{}' の削除に失敗しました: {}", path, err)
}

pub fn err_user_not_found(user: &str) -> String {
    format!("エラー: ユーザー '{}' が見つかりません", user)
}

pub fn err_group_not_found(group: &str) -> String {
    format!("エラー: グループ '{}' が見つかりません", group)
}

pub fn err_file_not_found(file: &str) -> String {
    format!("エラー: ファイル '{}' が見つかりません", file)
}

pub fn err_cannot_open_file(file: &str, err: &str) -> String {
    format!("エラー: ファイル '{}' を開けません: {}", file, err)
}

pub fn warn_cannot_read_dir(path: &str) -> String {
    format!("警告: ディレクトリ '{}' を読めません", path)
}

pub fn warn_cannot_stat(path: &str) -> String {
    format!("警告: '{}' の情報を取得できません", path)
}

pub fn prompt_exec(cmd: &str, file: &str) -> String {
    format!("< {} ... {} >? ", cmd, file)
}

pub fn err_ok_no_batch(opt: &str) -> String {
    format!(
        "エラー: {} は `{{}} +` 終端をサポートしません（`{{}} \\;` を使用してください）",
        opt
    )
}

pub fn err_exec_batch_partial_placeholder(arg: &str) -> String {
    format!(
        "エラー: '-exec ... {{}} +' では {{}} は単独トークンである必要がありますが '{}' が指定されました",
        arg
    )
}
