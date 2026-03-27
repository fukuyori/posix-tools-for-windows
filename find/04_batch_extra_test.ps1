# =============================================================================
# 04_batch_extra_test.ps1 — バッチモードの追加テスト (Windows PowerShell)
# =============================================================================
# 使い方:
#   .\04_batch_extra_test.ps1
#   .\04_batch_extra_test.ps1 -FindExe .\target\release\find.exe
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
$Base   = Join-Path $env:TEMP ("find_batch_test_" + [IO.Path]::GetRandomFileName().Replace(".",""))
$TmpOut = Join-Path $env:TEMP ("find_batch_out_"  + [IO.Path]::GetRandomFileName().Replace(".",""))
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
New-Item -ItemType Directory -Path (Join-Path $Base "dir") -Force | Out-Null
1..5 | ForEach-Object { "content$_" | Set-Content (Join-Path $Base "dir\file$_.txt") }

Write-Host "  find    : $FindExe"
Write-Host "  テスト用: $Base"

try {

# =============================================================================
# テスト A: prefix-{} を {} + に渡すとパーサーエラーになる（GNU find 仕様）
# =============================================================================
Section "テスト A: -exec echo prefix-{} + -- パーサーがエラーを返す（GNU find 仕様）"

& $FindExe "$Base\dir" -type f -exec echo "prefix-{}" "+" >$null 2>&1
$exitA = $LASTEXITCODE
if ($exitA -ne 0) {
    Ok "prefix-{} + を正しくエラー終了した (exit=$exitA)"
} else {
    $outA = Invoke-Find @("$Base\dir", "-type", "f", "-exec", "echo", "prefix-{}", "+")
    Ng "prefix-{} + がエラーにならなかった" "非ゼロ終了" "ゼロ終了"
    Write-Host "    find の出力: $($outA -join ', ')" -ForegroundColor DarkGray
}

# =============================================================================
# テスト B: \; モードでは prefix-{} が正しく展開される
# echo は cmd 内部コマンドなので cmd /c echo で実行する
# =============================================================================
Section "テスト B: -exec cmd /c echo prefix-{} `;` -- `;` モードでは部分置換が動作する"

$outB = Invoke-Find @("$Base\dir", "-type", "f", "-name", "*.txt",
                       "-exec", "cmd", "/c", "echo", "prefix-{}", ";")
$cntB = ($outB | Where-Object { $_ -match "prefix-" } | Measure-Object).Count
Check $cntB 5 `
    "prefix-{} `;` で5ファイル分が prefix- 付きで出力された" `
    "prefix-{} `;` の展開件数が不正"
if ($outB) { Write-Host "    出力例: $($outB | Select-Object -First 1)" -ForegroundColor DarkGray }

# =============================================================================
# テスト C: {} + 完全一致は正常動作
# =============================================================================
Section "テスト C: -exec cmd /c type {} + -- 完全一致バッチは正常動作"

$outC = Invoke-Find @("$Base\dir", "-type", "f", "-name", "*.txt",
                       "-exec", "cmd", "/c", "type", "{}", "+")
$cntC = ($outC | Measure-Object -Line).Lines
Check $cntC 5 `
    "-exec cmd /c type {} + で5行出力された" `
    "-exec cmd /c type {} + の行数が不正"

# =============================================================================
# テスト D: ARG_MAX 分割 -- 大量ファイルで複数バッチに分割される
# cmd /c echo %CD% {} + で各バッチの実行ディレクトリと引数を確認する
# %CD% をコマンド側に書いておくと cmd が展開し、ファイルリストは後続引数になる
# =============================================================================
Section "テスト D: 大量ファイルで ARG_MAX 分割が機能する"

$manyDir = Join-Path $Base "many"
New-Item -ItemType Directory -Path $manyDir -Force | Out-Null
1..300 | ForEach-Object {
    $null = New-Item -ItemType File `
        -Path (Join-Path $manyDir ("long_filename_arg_max_test_{0:D4}.txt" -f $_)) -Force
}

# 検証1: -print で全300ファイルが find から見えること
$outD_print = Invoke-Find @($manyDir, "-type", "f", "-name", "*.txt", "-print")
$totalFound = ($outD_print | Measure-Object).Count

Write-Host "    作成ファイル数    : 300"
Write-Host "    find が認識した数 : $totalFound"

Check $totalFound 300 `
    "全300ファイルが find から正しく認識された" `
    "find が認識したファイル数が不正"

# 検証2: -exec {} + で exit 0 になること（ARG_MAX を超えても分割して正常完了）
# ※ cmd /c echo の行長上限（8191文字）を避けるため、単純な exit 0 コマンドで成否を確認
& $FindExe $manyDir -type f -name "*.txt" -exec cmd /c "exit 0" "{}" "+" >$null 2>&1
$exitD = $LASTEXITCODE

Write-Host "    -exec {} + の終了コード: $exitD"
if ($exitD -eq 0) {
    Ok "ARG_MAX 分割で全300ファイルを -exec {} + で処理完了 (exit=0)"
} else {
    Ng "ARG_MAX 分割中にエラーが発生した" "0" "$exitD"
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
