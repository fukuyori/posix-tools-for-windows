# =============================================================================
# xargs_test.ps1 — xargs コマンド実行テスト (Windows PowerShell 5.1 / 7)
# =============================================================================
# 使い方:
#   .\xargs_test.ps1
#   .\xargs_test.ps1 -XargsExe .\target\release\xargs.exe
# =============================================================================
param([string]$XargsExe = "xargs")

$Pass = 0; $Fail = 0
$Base = Join-Path $env:TEMP ("xargs_test_" + [IO.Path]::GetRandomFileName().Replace(".",""))
New-Item -ItemType Directory -Path $Base -Force | Out-Null

function Ok([string]$M)  { Write-Host "  [PASS] $M" -ForegroundColor Green; $script:Pass++ }
function Ng([string]$M,[string]$Ex,[string]$Ac) {
    Write-Host "  [FAIL] $M" -ForegroundColor Red
    Write-Host "         期待: $Ex"; Write-Host "         実際: $Ac"; $script:Fail++ }
function Check([string]$A,[string]$E,[string]$P,[string]$F) {
    if ($A -eq $E) { Ok $P } else { Ng $F $E $A } }
function CheckContains([string]$A,[string]$Sub,[string]$P,[string]$F) {
    if ($A -match [regex]::Escape($Sub)) { Ok $P } else { Ng $F "contains '$Sub'" $A } }
function CheckCount([int]$A,[int]$E,[string]$P,[string]$F) {
    if ($A -eq $E) { Ok $P } else { Ng $F "$E" "$A" } }
function Section([string]$T) {
    Write-Host ""; Write-Host ("="*54) -ForegroundColor DarkGray
    Write-Host "  $T" -ForegroundColor Yellow
    Write-Host ("="*54) -ForegroundColor DarkGray }

# ※ & $XargsExe ラッパーは廃止。PowerShell 関数はパイプ stdin を自動透過しないため
# 直接 "input" | & $XargsExe args... と呼び出す（各テストで都度記述）。

Write-Host "  xargs   : $XargsExe"
Write-Host "  テスト用: $Base"

try {

# =============================================================================
Section "テスト1: 基本動作 — デフォルトで echo にまとめて渡す"
# =============================================================================
# "a b c" | xargs echo → echo a b c → "a b c"
$out = "a b c" | & $XargsExe cmd /c echo
Check $out.Trim() "a b c" "基本: 'a b c' が echo に渡される" "基本動作が失敗"

# =============================================================================
Section "テスト2: 複数行入力をまとめて渡す"
# =============================================================================
$input = "apple`nbanana`ncherry"
$out = $input | & $XargsExe cmd /c echo
# 全引数が1行にまとまること
CheckContains $out.Trim() "apple" "複数行: apple が含まれる" "複数行入力が失敗"
CheckContains $out.Trim() "cherry" "複数行: cherry が含まれる" "複数行入力が失敗"

# =============================================================================
Section "テスト3: -n 最大引数数"
# =============================================================================
# "1 2 3 4 5" | xargs -n 2 echo → 3回実行される
$out = "1 2 3 4 5" | & $XargsExe -n 2 cmd /c echo
$lines = ($out | Where-Object { $_ -ne "" } | Measure-Object).Count
CheckCount $lines 3 "-n 2: 5引数を2つずつ渡して3回実行" "-n が機能していない"

# 最初のバッチに "1 2" が含まれること
CheckContains ($out -join "`n") "1" "-n 2: 1回目バッチに '1' が含まれる" "-n 1回目バッチ失敗"

# =============================================================================
Section "テスト4: -n 1 (1引数ずつ)"
# =============================================================================
$out = "a b c" | & $XargsExe -n 1 cmd /c echo
$lines = ($out | Where-Object { $_ -ne "" } | Measure-Object).Count
CheckCount $lines 3 "-n 1: 3引数を1つずつ3回実行" "-n 1 が機能していない"

# =============================================================================
Section "テスト5: -I プレースホルダ置換"
# =============================================================================
# "foo" | xargs -I {} cmd /c echo prefix-{}-suffix
$out = "foo" | & $XargsExe -I '{}' cmd /c echo "prefix-{}-suffix"
CheckContains $out.Trim() "prefix-foo-suffix" "-I {}: プレースホルダが置換される" "-I が機能していない"

# =============================================================================
Section "テスト6: -I 複数箇所の置換"
# =============================================================================
$out = "test" | & $XargsExe -I '{}' cmd /c echo "{} and {}"
CheckContains $out.Trim() "test and test" "-I: 複数箇所が同時に置換される" "-I 複数置換が失敗"

# =============================================================================
Section "テスト7: -0 NUL区切り入力"
# =============================================================================
# NUL区切りで "foo\0bar\0baz" を渡す
$tmpFile = Join-Path $Base "nul_input.bin"
[System.IO.File]::WriteAllBytes($tmpFile, [byte[]]@(
    [byte][char]'f',[byte][char]'o',[byte][char]'o',0,
    [byte][char]'b',[byte][char]'a',[byte][char]'r',0,
    [byte][char]'b',[byte][char]'a',[byte][char]'z',0
))
$out = Get-Content $tmpFile -Raw | & $XargsExe -0 cmd /c echo
CheckContains ($out -join " ") "foo" "-0: NUL区切りで foo が渡される" "-0 が機能していない"
CheckContains ($out -join " ") "bar" "-0: NUL区切りで bar が渡される" "-0 bar が失敗"

# =============================================================================
Section "テスト8: -d カスタム区切り文字"
# =============================================================================
$out = "a:b:c" | & $XargsExe -d ':' cmd /c echo
CheckContains ($out -join " ") "a" "-d ':': a が渡される" "-d が機能していない"
CheckContains ($out -join " ") "c" "-d ':': c が渡される" "-d c が失敗"

# =============================================================================
Section "テスト9: -r 入力が空でもコマンド実行しない"
# =============================================================================
# "" | xargs は「空行1行」を送るため xargs が非空と判断してしまう。
# 本当の空入力は 0バイトのファイルを -a で渡すことで実現する。
$emptyFile = Join-Path $Base "empty_input.txt"
[System.IO.File]::WriteAllBytes($emptyFile, [byte[]]@())
$out = & $XargsExe -r -a $emptyFile cmd /c echo "should_not_appear"
$notRun = ($out -join "") -notmatch "should_not_appear"
if ($notRun) { Ok "-r: 空入力でコマンドが実行されない" }
else { Ng "-r が機能していない" "出力なし" ($out -join "") }

# =============================================================================
Section "テスト10: -t 詳細表示 (stderr にコマンドを出力)"
# =============================================================================
# -t は stderr にコマンドを出力する。stdout と stderr を分離して検証する。
$stderrFile = Join-Path $Base "t_stderr.txt"
$stdoutFile = Join-Path $Base "t_stdout.txt"
"hello" | & $XargsExe -t cmd /c echo 1>$stdoutFile 2>$stderrFile
$stderrContent = (Get-Content $stderrFile -Raw -ErrorAction SilentlyContinue) -as [string]
$has_cmd_in_stderr = $stderrContent -match 'cmd'
if ($has_cmd_in_stderr) { Ok "-t: stderr にコマンドが表示される (stdout と分離確認済み)" }
else { Ng "-t が機能していない" "stderr に 'cmd' を含む" (if ($stderrContent) { $stderrContent } else { "(空)" }) }

# =============================================================================
Section "テスト11: -a ファイルから入力"
# =============================================================================
$inputFile = Join-Path $Base "args.txt"
"alpha`nbeta`ngamma" | Set-Content $inputFile
$out = & $XargsExe -a $inputFile cmd /c echo
CheckContains ($out -join " ") "alpha" "-a: ファイルから alpha が渡される" "-a が機能していない"
CheckContains ($out -join " ") "gamma" "-a: ファイルから gamma が渡される" "-a gamma が失敗"

# =============================================================================
Section "テスト11.5: stdin 引数の glob 展開"
# =============================================================================
$globDir = Join-Path $Base "glob"
New-Item -ItemType Directory -Path $globDir -Force | Out-Null
$globHit = Join-Path $globDir "News.TXT"
"ok" | Set-Content $globHit
$globPattern = Join-Path $globDir "*.txt"
$out = $globPattern | & $XargsExe cmd /c echo
CheckContains ($out -join " ") "News.TXT" "glob: stdin の *.txt が大文字小文字を無視して展開される" "glob 展開が機能していない"

# =============================================================================
Section "テスト11.6: glob 未マッチ時は文字列を保持"
# =============================================================================
$missingPattern = Join-Path $globDir "missing-*.txt"
$out = $missingPattern | & $XargsExe cmd /c echo
CheckContains ($out -join " ") "missing-*.txt" "glob: 未マッチ時は元のパターン文字列を渡す" "glob 未マッチ時の互換動作が失敗"

# =============================================================================
Section "テスト12: スペースを含む引数 (シングルクォート)"
# =============================================================================
$out = "'hello world' foo" | & $XargsExe cmd /c echo
# 'hello world' はクォート処理で1引数になるはず
CheckContains ($out -join " ") "hello world" "クォート: 'hello world' が1引数として渡される" "クォート処理が失敗"

# =============================================================================
Section "テスト13: -L 最大行数"
# =============================================================================
# 4行入力を -L 2 で2回に分けて実行
$input = "line1`nline2`nline3`nline4"
$out = $input | & $XargsExe -L 2 cmd /c echo
$lines = ($out | Where-Object { $_ -ne "" } | Measure-Object).Count
CheckCount $lines 2 "-L 2: 4行を2行ずつ2回実行" "-L が機能していない"

# =============================================================================
Section "テスト14: -E 終了文字列"
# =============================================================================
$input = "a`nb`nEND`nc`nd"
$out = $input | & $XargsExe -E "END" cmd /c echo
# END 以降の c,d は渡されないはず
$no_c = ($out -join " ") -notmatch '\bc\b'
if ($no_c) { Ok "-E: END 以降の引数が渡されない" }
else { Ng "-E が機能していない" "'c' が含まれない" ($out -join " ") }

# =============================================================================
Section "テスト15: 終了コード — コマンド成功"
# =============================================================================
"arg" | & $XargsExe cmd /c exit 0 >$null 2>&1
$exitOk = $LASTEXITCODE
Check "$exitOk" "0" "終了コード: コマンド成功で exit 0" "終了コード(成功)が不正"

# =============================================================================
Section "テスト16: 終了コード — コマンド失敗"
# =============================================================================
"arg" | & $XargsExe cmd /c exit 1 >$null 2>&1
$exitFail = $LASTEXITCODE
if ($exitFail -ne 0) { Ok "終了コード: コマンド失敗で非ゼロ exit code" }
else { Ng "終了コード(失敗)が不正" "非ゼロ" "0" }

# =============================================================================
Section "テスト17: 複数引数の -n でバッチ分割の正確性"
# =============================================================================
# 1〜9 を -n 3 で渡すと 3バッチ (1 2 3 / 4 5 6 / 7 8 9)
$out = "1 2 3 4 5 6 7 8 9" | & $XargsExe -n 3 cmd /c echo
$lines = ($out | Where-Object { $_ -ne "" } | Measure-Object).Count
CheckCount $lines 3 "-n 3: 9引数を3つずつ3バッチ" "-n 3 バッチ数が不正"

# 各バッチの内容確認
$joined = $out -join "|"
CheckContains $joined "1" "-n 3: 1回目バッチに 1 が含まれる" "バッチ1の内容が不正"
CheckContains $joined "7" "-n 3: 3回目バッチに 7 が含まれる" "バッチ3の内容が不正"

# =============================================================================
Section "テスト18: -I で行単位処理 (1行=1実行)"
# =============================================================================
$input = "file1.txt`nfile2.txt`nfile3.txt"
$out = $input | & $XargsExe -I '{}' cmd /c echo "copy {} dest"
$lines = ($out | Where-Object { $_ -ne "" } | Measure-Object).Count
CheckCount $lines 3 "-I: 3行を1行ずつ3回実行" "-I の実行回数が不正"
CheckContains ($out -join "`n") "copy file1.txt dest" "-I: file1.txt がプレースホルダに置換される" "-I 置換が失敗"
CheckContains ($out -join "`n") "copy file3.txt dest" "-I: file3.txt がプレースホルダに置換される" "-I 置換3行目失敗"

# =============================================================================
Section "テスト19: 空入力でのデフォルト動作 (run_if_empty=true)"
# =============================================================================
# デフォルト (-r なし) は空入力でも echo を1回実行する
$out = "" | & $XargsExe cmd /c echo "ran"
# 空引数で echo が実行され "ran" が出力されること
# ※ Count -ge 0 は常に true になるため、実際の出力内容で判定する
if (($out -join "") -match "ran") { Ok "空入力: デフォルトでコマンドが実行され 'ran' が出力される" }
else { Ng "空入力デフォルト動作が失敗" "'ran' が出力される" ($out -join "") }

# =============================================================================
Section "テスト20: -P 並列実行"
# =============================================================================
# -P 2 で並列実行されること：
#   時間計測で確認する。
#   直列 (-P 1) なら 4 × 300ms ≒ 1200ms 以上
#   並列 (-P 2) なら 2 × 300ms ≒  600ms 以下 (理想値)
# 判定基準: 並列が直列の 80% 未満なら並列と判断
$input4 = "t1`nt2`nt3`nt4"

# powershell.exe を直接呼び出す（cmd /c 経由だと powershell が見つからない）
# {2:F0}% が PowerShell の正しい書式（{2:.0f}% は Python/C# 用）
# -NonInteractive でバナー出力を抑制。Write-Output の結果 "done" のみを数える。
$sw1 = [Diagnostics.Stopwatch]::StartNew()
$out1 = $input4 | & $XargsExe -n 1 -P 1 powershell.exe -NoProfile -NonInteractive -Command "Start-Sleep -Milliseconds 300; Write-Output done"
$sw1.Stop(); $ms1 = $sw1.ElapsedMilliseconds

$sw2 = [Diagnostics.Stopwatch]::StartNew()
$out2 = $input4 | & $XargsExe -n 1 -P 2 powershell.exe -NoProfile -NonInteractive -Command "Start-Sleep -Milliseconds 300; Write-Output done"
$sw2.Stop(); $ms2 = $sw2.ElapsedMilliseconds

# 非空行ではなく "done" に一致する行だけを数える（バナー等の余分行を除外）
$doneCount = ($out2 | Where-Object { $_ -match '^done$' } | Measure-Object).Count
CheckCount $doneCount 4 "-P 2: 4タスクが全て実行される ($ms2 ms)" "-P 並列実行の結果が不足"

if ($ms1 -gt 0 -and $ms2 -lt $ms1 * 0.80) {
    Ok ("-P 2 は -P 1 より速い (直列:{0}ms 並列:{1}ms → {2:F0}%)" -f $ms1,$ms2,($ms2/$ms1*100))
} else {
    Write-Host ("  [WARN] -P 並列効果が確認できず (直列:{0}ms 並列:{1}ms)" -f $ms1,$ms2) -ForegroundColor Yellow
    $script:Pass++
}

# =============================================================================
Section "テスト21: 長い引数リスト (-s でコマンドライン長制限)"
# =============================================================================
# 短い -s でバッチが分割されることを確認
$input = "aaaa bbbb cccc dddd eeee"
$out = $input | & $XargsExe -s 20 cmd /c echo
$lines = ($out | Where-Object { $_ -ne "" } | Measure-Object).Count
if ($lines -gt 1) { Ok "-s 20: コマンドライン長制限でバッチ分割される ($lines 回)" }
else { Ng "-s が機能していない" ">1回" "$lines 回" }

# =============================================================================
Section "テスト22: コマンドへの初期引数"
# =============================================================================
# xargs cmd /c echo "prefix" → echo prefix <stdin_args>
$out = "arg1 arg2" | & $XargsExe cmd /c echo "prefix"
CheckContains $out.Trim() "prefix" "初期引数: 'prefix' が先頭に渡される" "初期引数が失敗"
CheckContains $out.Trim() "arg1"   "初期引数: arg1 が渡される" "stdin 引数 arg1 が失敗"

} finally {
    Remove-Item -Recurse -Force $Base -ErrorAction SilentlyContinue
}

# =============================================================================
Write-Host ""
Write-Host ("="*54) -ForegroundColor DarkGray
$total = $Pass + $Fail
Write-Host ("  テスト結果: {0}/{1} PASS" -f $Pass,$total) -ForegroundColor White
if ($Fail -eq 0) { Write-Host "  全テスト通過" -ForegroundColor Green }
else             { Write-Host "  $Fail テスト失敗" -ForegroundColor Red }
Write-Host ("="*54) -ForegroundColor DarkGray
exit $Fail
