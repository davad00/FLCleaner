@echo off
echo Building FL Studio Backup Cleaner installer...

REM Check if Inno Setup is installed
if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" (
    set ISCC="%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
) else if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" (
    set ISCC="%ProgramFiles%\Inno Setup 6\ISCC.exe"
) else (
    echo Inno Setup not found. Please install Inno Setup 6 from https://jrsoftware.org/isdl.php
    echo After installation, run this script again.
    pause
    exit /b 1
)

REM Build the release version
echo Building release version...
cargo build --release
if %ERRORLEVEL% neq 0 (
    echo Failed to build release version.
    pause
    exit /b 1
)

REM Create installer directory if it doesn't exist
if not exist "installer" mkdir installer

REM Run Inno Setup compiler
echo Creating installer...
%ISCC% installer.iss
if %ERRORLEVEL% neq 0 (
    echo Failed to create installer.
    pause
    exit /b 1
)

echo.
echo Installer created successfully!
echo You can find the installer in the "installer" folder.
echo.

pause 