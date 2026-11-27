@echo off
rem Captures the frames of the README animations on Windows, without Python.
rem
rem Expects in target\readme-gifs\: demo\ with the KiCad project
rem (RoyalBlue54L-Feather.kicad_pcb and the files next to it) and one folder
rem per scene with steps.json, as written by
rem   python scripts\readme-gifs\record.py --emit target\readme-gifs <scenes>
rem The frames (f0000.png ...) are written into the scene folders;
rem   python scripts\readme-gifs\record.py --assemble target\readme-gifs <scenes>
rem makes the GIFs from them. kicad-cli is found as usual (PATH, the KiCad
rem installation, or KDRAFT_KICAD_CLI).
setlocal
cd /d "%~dp0..\.."
cargo build --release -p kdraft-ui
if errorlevel 1 exit /b 1
set "GUI=%CD%\target\release\kdraft-gui.exe"
set "WORK=%CD%\target\readme-gifs"
if not exist "%WORK%\demo\RoyalBlue54L-Feather.kicad_pcb" (
  echo %WORK%\demo\RoyalBlue54L-Feather.kicad_pcb is missing
  exit /b 1
)
for /d %%S in ("%WORK%\*") do if exist "%%S\steps.json" (
  echo == %%~nxS
  pushd "%%S"
  del /q f*.png last.png settings.json 2>nul
  "%GUI%" --screenshot last.png --size 1200x760 --theme light --settings settings.json --script steps.json ..\demo\RoyalBlue54L-Feather.kicad_pcb
  if errorlevel 1 echo FAILED: %%~nxS
  popd
)
echo done: frames are in %WORK%
