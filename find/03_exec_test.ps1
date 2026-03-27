# =============================================================================
# 03_exec_test.ps1 — -exec / -execdir / -ok のテスト (Windows PowerShell)
# =============================================================================
# 使い方:
#   .\03_exec_test.ps1                              # PATH 上の find を使用
#   .\03_exec_test.ps1 -FindExe .\target\release\find.exe
# =============================================================================
param(
    [string]$FindExe = "find"
)

# Windows の system32\find.exe と混在しないよう解決
if ($FindExe -eq "find") {
    $resolved = Get-Command find -ErrorAction SilentlyContinue |
                Where-Object { $_.Source -notlike "*System32*" } |
                Select-Object -First 1 -ExpandProperty Source
    if ($resolved) { $FindExe = $resolved }
}

$Pass   = 0
$Fail   = 0
$Base   = Join-Path $env:TEMP ("find_exec_test_"  + [IO.Path]::GetRandomFileName().Replace(".",""))
$TmpOut = Join-Path $env:TEMP ("find_exec_out_"   + [IO.Path]::GetRandomFileName().Replace(".",""))
New-Item -ItemType Directory -Path $Base, $TmpOut -Force | Out-Null

# ── ユーティリティ ─────────────────────────────────────────
function Ok([string]$Msg) {
    Write-Host "  [PASS] $Msg" -ForegroundColor Green
    $script:Pass++
}
function Ng([string]$Msg, [string]$Expected, [string]$Actual) {
    Write-Host "  [FAIL] $Msg" -ForegroundColor Red
    Write-Host "         期待: $Expected"
    Write-Host "         実際: $Actual"
    $script:Fail++
}
function Check([int]$Actual, [int]$Expected, [string]$PassMsg, [string]$FailMsg) {
    if ($Actual -eq $Expected) { Ok $PassMsg } else { Ng $FailMsg "$Expected" "$Actual" }
}
function Section([string]$Title) {
    Write-Host ""
    Write-Host ("=" * 54) -ForegroundColor DarkGray
    Write-Host "  $Title" -ForegroundColor Yellow
    Write-Host ("=" * 54) -ForegroundColor DarkGray
}
function Invoke-Find {
    param([string[]]$FindArgs)
    & $FindExe @FindArgs 2>$null
}

# ── テスト環境の構築 ───────────────────────────────────────
foreach ($sub in "dirA","dirB","dirC") {
    New-Item -ItemType Directory -Path (Join-Path $Base $sub) -Force | Out-Null
}
"alpha"   | Set-Content (Join-Path $Base "dirA\file1.txt")
"beta"    | Set-Content (Join-Path $Base "dirA\file2.txt")
"gamma"   | Set-Content (Join-Path $Base "dirB\file3.txt")
"delta"   | Set-Content (Join-Path $Base "dirB\file4.txt")
"epsilon" | Set-Content (Join-Path $Base "dirC\file5.txt")

Write-Host "  find    : $FindExe"
Write-Host "  テスト用: $Base"

try {

# =============================================================================
# テスト1: -exec {} \; 個別実行
# Windows では type は cmd 内部コマンドなので cmd /c type {} で実行する
# =============================================================================
Section "テスト1: -exec cmd /c type {} `;` -- ファイルごとにコマンドを実行"

$out1 = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                       "-exec", "cmd", "/c", "type", "{}", ";")
$cnt1 = ($out1 | Measure-Object -Line).Lines
Check $cnt1 5 `
    "-exec cmd /c type {} `;` で5ファイル分の内容が出力された ($cnt1 行)" `
    "-exec cmd /c type {} `;` の行数が不正"

# =============================================================================
# テスト2: -exec {} + バッチ実行
# =============================================================================
Section "テスト2: -exec cmd /c type {} + -- まとめてコマンドを実行"

$out2 = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                       "-exec", "cmd", "/c", "type", "{}", "+")
$cnt2 = ($out2 | Measure-Object -Line).Lines
Check $cnt2 5 `
    "-exec cmd /c type {} + で5ファイル分の内容がまとめて出力された ($cnt2 行)" `
    "-exec cmd /c type {} + の行数が不正"

# =============================================================================
# テスト2b: -exec {} \; の \; エスケープ表記を受理する
# =============================================================================
Section "テスト2b: -exec cmd /c type {} \\; -- エスケープ終端も受理する"

$out2b = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                        "-exec", "cmd", "/c", "type", "{}", "\;")
$cnt2b = ($out2b | Measure-Object -Line).Lines
Check $cnt2b 5 `
    "-exec cmd /c type {} \\; で5ファイル分の内容が出力された ($cnt2b 行)" `
    "-exec cmd /c type {} \\; の行数が不正"

# =============================================================================
# テスト3: -execdir {} \; カレントディレクトリが変わる
# cmd /c echo %CD% でカレントディレクトリを出力させる
# =============================================================================
Section "テスト3: -execdir cmd /c echo %CD% `;` -- ファイルのあるディレクトリで実行"

$dirs3  = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                         "-execdir", "cmd", "/c", "echo", "%CD%", ";")
$udirs3 = ($dirs3 | Sort-Object -Unique | Measure-Object).Count
Check $udirs3 3 `
    "-execdir cmd /c echo %CD% `;` が3ディレクトリで実行された" `
    "-execdir `;` のユニークディレクトリ数が不正"

# 引数がサブパスでなくファイル名のみであることを確認
$args3 = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                        "-execdir", "cmd", "/c", "echo", "{}", ";")
# .\ を除いたあとにパス区切りがないことを確認
$hasSep = $args3 | Where-Object { ($_ -replace '^\.\\','') -match '\\' }
if ($hasSep) {
    Ng "-execdir の引数にサブパスが含まれている" "ファイル名のみ" ($hasSep | Select-Object -First 1)
} else {
    Ok "-execdir の引数はファイル名のみ（.\file か file 形式）"
}

# =============================================================================
# テスト4: -execdir {} + バッチでもディレクトリが正しく適用される
# cmd /c echo %CD% を + バッチで実行 → 同一ディレクトリのファイルは1回の呼び出しにまとまる
# %CD% の展開は cmd に任せるため文字列として渡す
# =============================================================================
Section "テスト4: -execdir cmd /c echo %CD% {} + -- バッチ実行でもディレクトリが適用される"

$dirs4  = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                         "-execdir", "cmd", "/c", "echo", "%CD%", "{}", "+")
# バッチモードでは各ディレクトリで1回呼ばれるため、出力行数 = ディレクトリ数
# (%CD% は {} より前にあるため引数と混在しない)
$udirs4 = ($dirs4 | Sort-Object -Unique | Measure-Object).Count
Check $udirs4 3 `
    "-execdir cmd /c echo %CD% {} + で3ディレクトリが正しく分離された" `
    "-execdir バッチモードのディレクトリ数が不正"

# =============================================================================
# テスト5: -exec の終了コードが述語として機能する
# =============================================================================
Section "テスト5: -exec の終了コードが述語として機能する"

# 成功（exit 0）→ 後続の -print が実行される
$out5t = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                        "-exec", "cmd", "/c", "exit 0", ";", "-print")
$cnt5t = ($out5t | Measure-Object).Count
Check $cnt5t 5 `
    "-exec (成功) の後の -print が実行された ($cnt5t 件)" `
    "-exec (成功) の後の -print が実行されなかった"

# 失敗（exit 1）→ 後続の -print が実行されない
$out5f = Invoke-Find @($Base, "-type", "f", "-name", "*.txt",
                        "-exec", "cmd", "/c", "exit 1", ";", "-print")
$cnt5f = ($out5f | Measure-Object).Count
Check $cnt5f 0 `
    "-exec (失敗) の後の -print は実行されなかった（正しく false 評価）" `
    "-exec (失敗) が true として評価された"

# =============================================================================
# テスト5b: PowerShell で使いやすい \! を否定として受理する
# =============================================================================
Section "テスト5b: \\! を否定演算子として受理する"

$out5b = Invoke-Find @($Base, "\!", "-name", "*.txt")
$cnt5b = ($out5b | Measure-Object).Count
Check $cnt5b 3 `
    "\! -name *.txt で .txt 以外の3ディレクトリが出力された" `
    "\! を否定演算子として解釈できなかった"

# =============================================================================
# テスト6: -ok {} + はエラーになる
# =============================================================================
Section "テスト6: -ok cmd {} + はエラーとして拒否される"

$errMsg6 = & $FindExe $Base -type f -ok cmd /c "exit 0" "+" 2>&1
$exit6   = $LASTEXITCODE
if ($exit6 -ne 0) {
    Ok "-ok {} + は正しくエラー終了した (exit=$exit6)"
    Write-Host "    エラーメッセージ: $($errMsg6 -join ' ')" -ForegroundColor DarkGray
} else {
    Ng "-ok {} + がエラーにならなかった" "非ゼロ終了" "ゼロ終了"
}

# =============================================================================
# テスト7: -ok の y/n 応答
# copy は cmd 内部コマンドなので cmd /c copy で実行する
# =============================================================================
Section "テスト7: -ok で 'y' 入力時のみ実行（自動応答）"

$copy1 = Join-Path $TmpOut "test7_y.txt"
"y" | & $FindExe (Join-Path $Base "dirA") -maxdepth 1 -type f -name "file1.txt" `
        -ok cmd /c copy "{}" $copy1 ";" 2>$null | Out-Null
if (Test-Path $copy1) {
    Ok "-ok で 'y' を入力するとコマンドが実行された"
} else {
    Ng "-ok で 'y' を入力したがコマンドが実行されなかった" "ファイル生成" "なし"
}

$copy2 = Join-Path $TmpOut "test7_n.txt"
"n" | & $FindExe (Join-Path $Base "dirA") -maxdepth 1 -type f -name "file2.txt" `
        -ok cmd /c copy "{}" $copy2 ";" 2>$null | Out-Null
if (-not (Test-Path $copy2)) {
    Ok "-ok で 'n' を入力するとコマンドは実行されなかった"
} else {
    Ng "-ok で 'n' を入力したがコマンドが実行されてしまった" "ファイルなし" "ファイル生成"
}

} finally {
    Remove-Item -Recurse -Force $Base, $TmpOut -ErrorAction SilentlyContinue
}

# =============================================================================
# 結果サマリー
# =============================================================================
Write-Host ""
Write-Host ("=" * 54) -ForegroundColor DarkGray
$total = $Pass + $Fail
Write-Host ("  テスト結果: {0}/{1} PASS" -f $Pass, $total) -ForegroundColor White
if ($Fail -eq 0) {
    Write-Host "  全テスト通過" -ForegroundColor Green
} else {
    Write-Host "  $Fail テスト失敗" -ForegroundColor Red
}
Write-Host ("=" * 54) -ForegroundColor DarkGray
exit $Fail
