@echo off
rem WorldMaker launcher: builds the app in release mode and runs it.
rem First build takes a few minutes; later launches are quick.
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust is not installed or not on PATH.
    echo Install it from https://rustup.rs and run this file again.
    pause
    exit /b 1
)

echo Building WorldMaker (release)...
cargo build --release -p worldmaker-app
if errorlevel 1 (
    echo.
    echo Build failed. The full error is above.
    pause
    exit /b 1
)

start "" "target\release\worldmaker-app.exe"
endlocal
