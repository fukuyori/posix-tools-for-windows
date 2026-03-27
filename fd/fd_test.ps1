# =============================================================================
# fd_test.ps1 — fd コマンド オプション総合テスト (Windows PowerShell 5.1 / 7)
# =============================================================================
# 使い方:
#   .\fd_test.ps1
#   .\fd_test.ps1 -FdExe .\target\release\fd.exe
# =============================================================================
param([string]$FdExe = "fd")

$Pass = 0; $Fail = 0
$Base = Join-Path $env:TEMP ("fd_test_" + [IO.Path]::GetRandomFileName().Replace(".",""))
$Out  = Join-Path $env:TEMP ("fd_out_"  + [IO.Path]::GetRandomFileName().Replace(".",""))
New-Item -ItemType Directory -Path $Base,$Out -Force | Out-Null

function Ok([string]$M) { Write-Host "  [PASS] $M" -ForegroundColor Green; $script:Pass++ }
function Ng([string]$M,[string]$Ex,[string]$Ac) {
    Write-Host "  [FAIL] $M" -ForegroundColor Red
    Write-Host "         期待: $Ex"; Write-Host "         実際: $Ac"; $script:Fail++ }
function Check([int]$A,[int]$E,[string]$P,[string]$F) {
    if ($A -eq $E) { Ok $P } else { Ng $F "$E" "$A" } }
function CheckGt([int]$A,[int]$T,[string]$P,[string]$F) {
    if ($A -gt $T) { Ok "$P ($A)" } else { Ng $F ">$T" "$A" } }
function CheckTrue([bool]$A,[string]$P,[string]$F) {
    if ($A) { Ok $P } else { Ng $F "true" "false" } }
function Section([string]$T) {
    Write-Host ""; Write-Host ("="*54) -ForegroundColor DarkGray
    Write-Host "  $T" -ForegroundColor Yellow
    Write-Host ("="*54) -ForegroundColor DarkGray }

# fd を実行。fd の引数順: fd [OPTIONS] [PATTERN] [PATH]
# パターン不要な場合は '.' を第1引数に渡すこと（全名称にマッチ）
function RunFd { & $FdExe @args 2>$null }

# =============================================================================
# テスト環境構築
# =============================================================================
# 非hidden エントリ: ファイル8 + ディレクトリ4 = 計12
#   ファイル: file_a.txt file_b.log File_C.TXT empty.txt big.bin
#             subdir/nested.txt  subdir/deep/deepfile.rs
#             "space dir/space file.txt"
#   ディレクトリ: subdir  subdir/deep  exec_out  "space dir"

$dirs = @("subdir","subdir\deep",".hidden_dir","space dir","exec_out")
foreach ($d in $dirs) { New-Item -ItemType Directory -Path (Join-Path $Base $d) -Force | Out-Null }

"content_a" | Set-Content (Join-Path $Base "file_a.txt")
"content_b" | Set-Content (Join-Path $Base "file_b.log")
"content_C" | Set-Content (Join-Path $Base "File_C.TXT")
""          | Set-Content (Join-Path $Base "empty.txt")
"nested"    | Set-Content (Join-Path $Base "subdir\nested.txt")
"deep"      | Set-Content (Join-Path $Base "subdir\deep\deepfile.rs")
"hidden"    | Set-Content (Join-Path $Base ".hidden_file")
"inhidden"  | Set-Content (Join-Path $Base ".hidden_dir\inside_hidden.txt")
"space"     | Set-Content (Join-Path $Base "space dir\space file.txt")

# big.bin: 2MB ゼロバイト列 (PS5/PS7 両対応)
$bigBin = Join-Path $Base "big.bin"
[System.IO.File]::WriteAllBytes($bigBin, [byte[]]::new(2MB))

Write-Host "  fd      : $FdExe"
Write-Host "  テスト用: $Base"

try {

# =============================================================================
Section "テスト1: パターン '.' — 全非hiddenエントリ列挙"
# =============================================================================
$r = RunFd '.' $Base
Check ($r | Measure-Object).Count 12 "全エントリ: 12件 (ファイル8+ディレクトリ4)" "全エントリ件数が不正"

# =============================================================================
Section "テスト2: 正規表現パターン"
# =============================================================================
# .txt で終わるファイル: file_a.txt File_C.TXT empty.txt nested.txt "space file.txt" = 5件
$r = RunFd '\.txt$' $Base
Check ($r | Measure-Object).Count 5 "正規表現 \.txt : 5件マッチ" "正規表現マッチ件数が不正"

# =============================================================================
Section "テスト3: -g glob パターン"
# =============================================================================
$r = RunFd -g '*.txt' $Base
Check ($r | Measure-Object).Count 5 "-g '*.txt': 5件" "-g glob 件数が不正"

# =============================================================================
Section "テスト4: -F リテラル検索"
# =============================================================================
$r = RunFd -F 'file_a' $Base
Check ($r | Measure-Object).Count 1 "-F 'file_a': 1件" "-F リテラル件数が不正"

# =============================================================================
Section "テスト5: -t タイプフィルタ"
# =============================================================================
Check (RunFd -t f '.' $Base | Measure-Object).Count 8 "-t f: ファイル8件" "-t f 件数が不正"
Check (RunFd -t d '.' $Base | Measure-Object).Count 4 "-t d: ディレクトリ4件" "-t d 件数が不正"
Check (RunFd -t e '.' $Base | Measure-Object).Count 1 "-t e: 空ファイル1件 (empty.txt)" "-t e 件数が不正"

# =============================================================================
Section "テスト6: -e 拡張子フィルタ"
# =============================================================================
Check (RunFd -e txt '.' $Base | Measure-Object).Count 5 "-e txt: 5件" "-e txt 件数が不正"
Check (RunFd -e rs  '.' $Base | Measure-Object).Count 1 "-e rs: 1件 (deepfile.rs)" "-e rs 件数が不正"

# =============================================================================
Section "テスト7: -H 隠しファイル表示"
# =============================================================================
$cnt_n = (RunFd '.'    $Base | Measure-Object).Count
$cnt_h = (RunFd -H '.' $Base | Measure-Object).Count
CheckGt $cnt_h $cnt_n "-H: 隠しファイル含む件数が通常より多い" "-H が機能していない"
Check (RunFd -H '.' $Base | Where-Object { $_ -match '\.hidden_file' } | Measure-Object).Count `
    1 "-H: .hidden_file が列挙された" "-H: .hidden_file が見つからない"

# =============================================================================
Section "テスト8: --max-depth 深さ制限"
# =============================================================================
# depth 0 = 検索パス自体のみ。fd は start path 自体をエントリ出力しない仕様なので 0件が正しい。
# depth 1 = $Base 直下: ファイル5 + ディレクトリ3 = 8件
# depth 2 = さらに subdir/* が追加される
$cnt0 = (RunFd --max-depth 0 '.' $Base | Measure-Object).Count
$cnt1 = (RunFd --max-depth 1 '.' $Base | Measure-Object).Count
$cnt2 = (RunFd --max-depth 2 '.' $Base | Measure-Object).Count
Check  $cnt0 0  "--max-depth 0: 0件 (fd は start path 自体をリストしない)" "--max-depth 0 件数が不正"
Check  $cnt1 8  "--max-depth 1: ルート直下 8件 (ファイル5+ディレクトリ3)" "--max-depth 1 件数が不正"
CheckGt $cnt2 $cnt1 "--max-depth 2: depth 1 より多い (deepfile.rs 追加)" "--max-depth 2 が機能していない"

# =============================================================================
Section "テスト9: --min-depth 最小深さ"
# =============================================================================
$r = RunFd --min-depth 2 '.' $Base
Check ($r | Where-Object { $_ -match 'deepfile' } | Measure-Object).Count `
    1 "--min-depth 2: deepfile.rs が含まれる" "--min-depth が機能していない"
Check ($r | Where-Object { $_ -match 'file_a'   } | Measure-Object).Count `
    0 "--min-depth 2: ルート直下 file_a.txt は除外" "--min-depth 2 でルートが混入"

# =============================================================================
Section "テスト10: 大文字小文字（スマートケース / -s / -i）"
# =============================================================================
$r = RunFd 'file_c' $Base
Check ($r | Where-Object { $_ -match 'File_C' } | Measure-Object).Count `
    1 "スマートケース: 小文字 'file_c' で File_C.TXT がマッチ" "スマートケースが機能していない"

Check (RunFd -s 'file_c' $Base | Measure-Object).Count `
    0 "-s: 'file_c' はマッチなし (File_C.TXT は除外)" "-s が機能していない"

Check (RunFd -i 'FILE_C' $Base | Where-Object { $_ -match 'File_C' } | Measure-Object).Count `
    1 "-i: 'FILE_C' で File_C.TXT がマッチ" "-i が機能していない"

# =============================================================================
Section "テスト11: -E / --exclude 除外パターン"
# =============================================================================
$cnt_all = (RunFd '.'             $Base | Measure-Object).Count
$cnt_exc = (RunFd --exclude '*.log' '.' $Base | Measure-Object).Count
CheckGt $cnt_all $cnt_exc "--exclude '*.log': log を除外して件数が減る" "--exclude が機能していない"
Check (RunFd --exclude '*.log' '.' $Base | Where-Object { $_ -match '\.log$' } | Measure-Object).Count `
    0 "--exclude: log ファイルが結果に含まれない" "--exclude で log が混入"

# =============================================================================
Section "テスト12: -S サイズフィルタ"
# =============================================================================
Check (RunFd --size '+1Mi' '.' $Base | Where-Object { $_ -match 'big\.bin'   } | Measure-Object).Count `
    1 "--size +1Mi: big.bin (2MB) がマッチ" "--size +1Mi が機能していない"
Check (RunFd --size '-10b' '.' $Base | Where-Object { $_ -match 'empty\.txt' } | Measure-Object).Count `
    1 "--size -10b: empty.txt がマッチ" "--size -10b が機能していない"
Check (RunFd --size '-10b' '.' $Base | Where-Object { $_ -match 'big\.bin'   } | Measure-Object).Count `
    0 "--size -10b: big.bin は除外" "--size で big.bin が混入"

# =============================================================================
Section "テスト13: -a 絶対パス"
# =============================================================================
$r = RunFd -a 'file_a' $Base
Check ($r | Where-Object { $_ -match [regex]::Escape($Base) } | Measure-Object).Count `
    1 "-a: 絶対パスで出力される" "-a が機能していない"

# =============================================================================
Section "テスト14: --max-results 件数制限"
# =============================================================================
Check (RunFd --max-results 2 '.' $Base | Measure-Object).Count `
    2 "--max-results 2: 2件のみ返す" "--max-results が機能していない"

# =============================================================================
Section "テスト15: -q / --quiet 終了コード"
# =============================================================================
& $FdExe 'file_a'               $Base --quiet >$null 2>&1; $ex1 = $LASTEXITCODE
& $FdExe 'no_such_pattern_xyz'  $Base --quiet >$null 2>&1; $ex2 = $LASTEXITCODE
Check $ex1 0 "--quiet: マッチあり → exit 0" "--quiet マッチあり exit code が不正"
Check $ex2 1 "--quiet: マッチなし → exit 1" "--quiet マッチなし exit code が不正"

# =============================================================================
Section "テスト16: --and 複数パターン AND"
# =============================================================================
# パターン='file' AND --and='\.txt$' → file_a.txt のみ。file_b.log は .txt でない。
# 単一引用符内の $ はリテラル → fd に正規表現アンカーとして渡る
$r = RunFd --and '\.txt$' 'file' $Base
Check ($r | Where-Object { $_ -match 'file_a\.txt' } | Measure-Object).Count `
    1 "--and: 'file' AND '\.txt' → file_a.txt がマッチ" "--and が機能していない"
Check ($r | Where-Object { $_ -match 'file_b\.log' } | Measure-Object).Count `
    0 "--and: file_b.log は .txt でないので除外" "--and で .log が混入"

# =============================================================================
Section "テスト17: -p フルパス検索"
# =============================================================================
Check (RunFd -p 'subdir.*txt' $Base | Where-Object { $_ -match 'nested\.txt' } | Measure-Object).Count `
    1 "-p 'subdir.*txt': nested.txt がマッチ" "-p が機能していない"

# =============================================================================
Section "テスト18: --format 出力フォーマット"
# =============================================================================
# {/} = basename
Check (RunFd --format '{/}' 'file_a' $Base | Where-Object { $_ -match '^file_a\.txt$' } | Measure-Object).Count `
    1 "--format '{/}': basename のみ出力" "--format {/} が機能していない"
# {/.} = basename 拡張子なし
Check (RunFd --format '{/.}' 'file_a' $Base | Where-Object { $_ -eq 'file_a' } | Measure-Object).Count `
    1 "--format '{/.}': 拡張子なし basename を出力" "--format {/.} が機能していない"

# =============================================================================
Section "テスト19: -x / --exec 各ファイルへのコマンド実行"
# =============================================================================
$execDir = Join-Path $Base "exec_out"

# .txt ファイルを exec_out にコピー
# デスティネーションをディレクトリ ($execDir) のみ指定。
# cmd copy は dest がディレクトリの場合、元のファイル名を保持してコピーする。
# 注意: '{/}' を Join-Path で組み合わせると fd プレースホルダではなく
#       リテラル文字列 '...\exec_out\{/}' になってしまうため使用しない。
RunFd '\.txt$' $Base -x cmd /c copy '{}' $execDir ';' | Out-Null
Check (Get-ChildItem $execDir -File | Measure-Object).Count `
    5 "-x copy: 5つの .txt ファイルがコピーされた" "-x copy の実行件数が不正"

# プレースホルダなし: path が末尾引数として直接渡る
$echoOut = RunFd -e rs '.' $Base -x cmd /c echo ';'
CheckGt ($echoOut | Where-Object { $_ -match 'deepfile' } | Measure-Object).Count `
    0 "-x echo (プレースホルダなし): deepfile.rs が出力される" "-x プレースホルダなしが機能していない"

# =============================================================================
Section "テスト20: -X / --exec-batch 全ファイルをまとめて実行"
# =============================================================================
# 正しい動作: '{}' に全 .txt パスがまとめて代入され echo が 1〜2 回実行
# 旧バグ: "echo path1 echo path2 ..." のように echo がファイル数回繰り返された
$batchOut = RunFd '\.txt$' $Base -X cmd /c echo '{}'
$lines = ($batchOut | Measure-Object).Count
$linesOk = if ($lines -le 2) { 1 } else { 0 }
Check $linesOk 1 "-X: 出力 1〜2 行 (echo が複数回繰り返されない)" "-X: 行数が多すぎる (旧バグの可能性)"

$joined = $batchOut -join " "
CheckTrue ($joined -match 'file_a' -and $joined -match 'nested') `
    "-X: 1回の実行に複数ファイルが含まれる" "-X: 複数ファイルがまとめて渡されていない"

# =============================================================================
Section "テスト21: -X + {/} (バッチ + basename 展開)"
# =============================================================================
$r = RunFd '\.txt$' $Base -X cmd /c echo '{/}'
$joined = $r -join " "
CheckTrue ($joined -match 'file_a\.txt' -and $joined -match 'nested\.txt') `
    "-X '{/}': 複数 basename が展開される" "-X '{/}' 展開が機能していない"

# =============================================================================
Section "テスト22: スペースを含むパスの -x 実行"
# =============================================================================
$spaceDst = Join-Path $Out "space_copy.txt"
RunFd 'space file' $Base -x cmd /c copy '{}' $spaceDst ';' | Out-Null
CheckTrue (Test-Path $spaceDst) "-x: スペース含むパスが正しくコピーされた" "-x: スペースパスのコピーに失敗"

# =============================================================================
Section "テスト23: exec 失敗時の終了コード"
# =============================================================================
& $FdExe 'file_a' $Base -x cmd /c "exit 1" ';' >$null 2>&1
CheckTrue ($LASTEXITCODE -ne 0) "-x: コマンド失敗時に非ゼロ exit code を返す" "-x: 常に exit 0 を返している"

# =============================================================================
Section "テスト24: --changed-within / --changed-before 時刻フィルタ"
# =============================================================================
CheckGt (RunFd --changed-within 1h '.' $Base | Measure-Object).Count `
    4 "--changed-within 1h: 直前に作成した全ファイルが対象" "--changed-within が機能していない"
CheckGt (RunFd --changed-before '2099-12-31' '.' $Base | Measure-Object).Count `
    4 "--changed-before 2099-12-31: 全ファイルが対象" "--changed-before が機能していない"

# =============================================================================
Section "テスト25: --one-file-system"
# =============================================================================
CheckGt (RunFd --one-file-system '.' $Base | Measure-Object).Count `
    0 "--one-file-system: エラーなく結果を返す" "--one-file-system でクラッシュ"

} finally {
    Remove-Item -Recurse -Force $Base,$Out -ErrorAction SilentlyContinue
}

# =============================================================================
Write-Host ""
Write-Host ("="*54) -ForegroundColor DarkGray
Write-Host ("  テスト結果: {0}/{1} PASS" -f $Pass,($Pass+$Fail)) -ForegroundColor White
if ($Fail -eq 0) { Write-Host "  全テスト通過" -ForegroundColor Green }
else             { Write-Host "  $Fail テスト失敗" -ForegroundColor Red }
Write-Host ("="*54) -ForegroundColor DarkGray
exit $Fail
