@echo off
setlocal EnableExtensions
cd /d "%~dp0"

echo === BlackShark Battery build ===

set "VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
if not exist "%VCVARS%" set "VCVARS=C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
if not exist "%VCVARS%" set "VCVARS=C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat"
if not exist "%VCVARS%" (
  echo ERROR: vcvars64.bat not found. Install VS Build Tools with C++ workload.
  exit /b 1
)

call "%VCVARS%" >nul
if errorlevel 1 (
  echo ERROR: failed to load MSVC environment.
  exit /b 1
)

if not exist publish mkdir publish
if not exist app.ico (
  where python >nul 2>&1 && python make_icon.py
)

echo [1/2] Compiling resources...
rc /nologo /fo publish\resource.res resource.rc
if errorlevel 1 exit /b 1

echo [2/2] Compiling main.cpp ...
cl /nologo /O2 /DUNICODE /D_UNICODE /W3 /EHsc /std:c++17 /utf-8 ^
  main.cpp /Fe:publish\BlackSharkBattery.exe /Fo:publish\ ^
  /link /SUBSYSTEM:WINDOWS /OPT:REF /OPT:ICF ^
  publish\resource.res ^
  setupapi.lib hid.lib shell32.lib gdi32.lib gdiplus.lib user32.lib advapi32.lib comctl32.lib ole32.lib

if errorlevel 1 (
  echo BUILD FAILED
  exit /b 1
)

echo.
echo OK: %~dp0publish\BlackSharkBattery.exe
for %%I in (publish\BlackSharkBattery.exe) do echo Size: %%~zI bytes
endlocal
