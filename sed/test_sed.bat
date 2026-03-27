@echo off
setlocal enabledelayedexpansion

REM sed テストスイート for Windows
REM 使い方: test_sed.bat [sed.exeのパス]

set SED=%1
if "%SED%"=="" set SED=sed.exe

set PASSED=0
set FAILED=0

echo === sed テストスイート ===
echo.

REM テスト用ファイル作成
echo line1> test_input.txt
echo line2>> test_input.txt
echo line3>> test_input.txt

echo foo bar foo> test_foo.txt
echo baz foo qux>> test_foo.txt

echo a> test_5lines.txt
echo b>> test_5lines.txt
echo c>> test_5lines.txt
echo d>> test_5lines.txt
echo e>> test_5lines.txt

echo --- 基本置換テスト ---

REM Test 1: 基本置換
echo foo| %SED% "s/foo/bar/" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="bar" (
    echo PASS: s/foo/bar/ 基本置換
    set /a PASSED+=1
) else (
    echo FAIL: s/foo/bar/ 基本置換 - got "!RESULT!"
    set /a FAILED+=1
)

REM Test 2: グローバル置換
echo foo foo foo| %SED% "s/foo/bar/g" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="bar bar bar" (
    echo PASS: s/foo/bar/g グローバル置換
    set /a PASSED+=1
) else (
    echo FAIL: s/foo/bar/g グローバル置換 - got "!RESULT!"
    set /a FAILED+=1
)

REM Test 3: 大文字小文字無視
echo foo| %SED% "s/FOO/bar/i" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="bar" (
    echo PASS: s/FOO/bar/i 大文字小文字無視
    set /a PASSED+=1
) else (
    echo FAIL: s/FOO/bar/i - got "!RESULT!"
    set /a FAILED+=1
)

REM Test 4: 別の区切り文字
echo foo| %SED% "s#foo#bar#" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="bar" (
    echo PASS: s#foo#bar# 別区切り文字
    set /a PASSED+=1
) else (
    echo FAIL: s#foo#bar# - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 後方参照テスト ---

REM Test 5: & 全体マッチ
echo foo| %SED% "s/foo/[&]/" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="[foo]" (
    echo PASS: ^& 全体マッチ
    set /a PASSED+=1
) else (
    echo FAIL: ^& 全体マッチ - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 削除コマンド ^(d^) ---

REM Test 6: d 削除
echo foo| %SED% "d" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="" (
    echo PASS: d 全行削除
    set /a PASSED+=1
) else (
    echo FAIL: d 全行削除 - got "!RESULT!"
    set /a FAILED+=1
)

REM Test 7: 2d 2行目削除
%SED% "2d" test_input.txt > test_out.txt
findstr /n "." test_out.txt | find /c ":" > test_count.txt
set /p COUNT=<test_count.txt
if "!COUNT!"=="2" (
    echo PASS: 2d 2行目削除
    set /a PASSED+=1
) else (
    echo FAIL: 2d 2行目削除 - got !COUNT! lines
    set /a FAILED+=1
)

echo.
echo --- 出力コマンド ^(p^) ---

REM Test 8: -n p
echo foo| %SED% -n "p" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="foo" (
    echo PASS: -n p 選択出力
    set /a PASSED+=1
) else (
    echo FAIL: -n p - got "!RESULT!"
    set /a FAILED+=1
)

REM Test 9: -n 2p
%SED% -n "2p" test_input.txt > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="line2" (
    echo PASS: -n 2p 2行目のみ
    set /a PASSED+=1
) else (
    echo FAIL: -n 2p - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- アドレス指定テスト ---

REM Test 10: 1 最初の行
%SED% "1s/line/LINE/" test_input.txt > test_out.txt
findstr "LINE1" test_out.txt > nul
if !errorlevel!==0 (
    echo PASS: 1s 最初の行
    set /a PASSED+=1
) else (
    echo FAIL: 1s 最初の行
    set /a FAILED+=1
)

REM Test 11: $ 最後の行
%SED% "$s/3/THREE/" test_input.txt > test_out.txt
findstr "lineTHREE" test_out.txt > nul
if !errorlevel!==0 (
    echo PASS: $s 最後の行
    set /a PASSED+=1
) else (
    echo FAIL: $s 最後の行
    set /a FAILED+=1
)

echo.
echo --- 否定アドレス ^(!^) ---

REM Test 12: 1!d 1行目以外削除
%SED% "1!d" test_input.txt > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="line1" (
    echo PASS: 1!d 1行目以外削除
    set /a PASSED+=1
) else (
    echo FAIL: 1!d - got "!RESULT!"
    set /a FAILED+=1
)

REM Test 13: $!d 最終行以外削除
%SED% "$!d" test_input.txt > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="line3" (
    echo PASS: $!d 最終行以外削除
    set /a PASSED+=1
) else (
    echo FAIL: $!d - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 文字変換 ^(y^) ---

REM Test 14: y/abc/ABC/
echo abcdef| %SED% "y/abc/ABC/" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="ABCdef" (
    echo PASS: y/abc/ABC/
    set /a PASSED+=1
) else (
    echo FAIL: y/abc/ABC/ - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 複数行処理 ^(N^) ---

REM Test 15: N 結合（改行削除）
%SED% ":a;N;$!ba;s/\n//g" test_5lines.txt > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="abcde" (
    echo PASS: :a;N;$!ba;s/\n//g 改行削除
    set /a PASSED+=1
) else (
    echo FAIL: 改行削除 - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- ホールドスペース ---

REM Test 16: h;g
echo foo| %SED% "h;g" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="foo" (
    echo PASS: h;g ホールド/ゲット
    set /a PASSED+=1
) else (
    echo FAIL: h;g - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 行番号 ^(=^) ---

REM Test 17: =
%SED% -n "=" test_input.txt > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="1" (
    echo PASS: = 行番号出力
    set /a PASSED+=1
) else (
    echo FAIL: = - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 複合テスト ---

REM Test 18: 複数コマンド
echo ab| %SED% "s/a/A/;s/b/B/" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="AB" (
    echo PASS: 複数コマンド
    set /a PASSED+=1
) else (
    echo FAIL: 複数コマンド - got "!RESULT!"
    set /a FAILED+=1
)

echo.
echo --- 挿入/追加 ---

REM Test 19: c change
echo foo| %SED% "c NEW" > test_out.txt
set /p RESULT=<test_out.txt
if "!RESULT!"=="NEW" (
    echo PASS: c change
    set /a PASSED+=1
) else (
    echo FAIL: c change - got "!RESULT!"
    set /a FAILED+=1
)

REM クリーンアップ
del /q test_input.txt test_foo.txt test_5lines.txt 2>nul
del /q test_out.txt test_count.txt 2>nul

echo.
echo === テスト結果 ===
echo Passed: %PASSED%
echo Failed: %FAILED%
set /a TOTAL=%PASSED%+%FAILED%
echo Total: %TOTAL%

if %FAILED%==0 (
    echo.
    echo All tests passed!
    exit /b 0
) else (
    echo.
    echo Some tests failed.
    exit /b 1
)
