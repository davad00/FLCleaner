@echo off
echo Creating FL Studio Backup Cleaner ZIP package...

REM Set version
set VERSION=1.0.0
set ZIP_FILE=FLCleaner_%VERSION%.zip

REM Build the release version
echo Building release version...
cargo build --release
if %ERRORLEVEL% neq 0 (
    echo Failed to build release version.
    pause
    exit /b 1
)

REM Create distribution directory
echo Creating distribution directory...
if exist "dist" rmdir /s /q dist
mkdir dist

REM Copy files to distribution directory
echo Copying files...
copy "target\release\flcleaner.exe" "dist\"
copy "README.md" "dist\"
copy "icon\favicon.ico" "dist\"

REM Delete existing ZIP file if it exists
if exist "%ZIP_FILE%" del "%ZIP_FILE%"

REM Create a simple batch file that will create the ZIP
echo Creating ZIP creation script...
echo Set objArgs = WScript.Arguments > create_zip.vbs
echo InputFolder = objArgs(0) >> create_zip.vbs
echo ZipFile = objArgs(1) >> create_zip.vbs
echo.>> create_zip.vbs
echo CreateZipFile InputFolder, ZipFile >> create_zip.vbs
echo.>> create_zip.vbs
echo Sub CreateZipFile(InputFolder, ZipFile) >> create_zip.vbs
echo   Set fso = CreateObject("Scripting.FileSystemObject") >> create_zip.vbs
echo   InputFolder = fso.GetAbsolutePathName(InputFolder) >> create_zip.vbs
echo.>> create_zip.vbs
echo   Set objShell = CreateObject("Shell.Application") >> create_zip.vbs
echo.>> create_zip.vbs
echo   Set objSource = objShell.NameSpace(InputFolder) >> create_zip.vbs
echo   if fso.FileExists(ZipFile) Then fso.DeleteFile ZipFile >> create_zip.vbs
echo.>> create_zip.vbs
echo   Set zip = fso.CreateTextFile(ZipFile, True) >> create_zip.vbs
echo   zip.Write "PK" ^& Chr(5) ^& Chr(6) ^& String(18, Chr(0)) >> create_zip.vbs
echo   zip.Close >> create_zip.vbs
echo.>> create_zip.vbs
echo   Set objZip = objShell.NameSpace(fso.GetAbsolutePathName(ZipFile)) >> create_zip.vbs
echo.>> create_zip.vbs
echo   objZip.CopyHere objSource.Items >> create_zip.vbs
echo.>> create_zip.vbs
echo   WScript.Sleep 5000 >> create_zip.vbs
echo End Sub >> create_zip.vbs

REM Execute the VBS script to create the ZIP
echo Creating ZIP file...
cscript //nologo create_zip.vbs dist %ZIP_FILE%

REM Clean up
del create_zip.vbs

REM Check if ZIP was created successfully
if not exist %ZIP_FILE% (
    echo Failed to create ZIP file.
    pause
    exit /b 1
)

echo.
echo ZIP package created successfully!
echo You can find the ZIP file in the current directory: %ZIP_FILE%
echo.

pause 