@echo off
chcp 65001 >nul
echo === Gestore File CP 1.0 ===
echo Compilazione in corso (la prima volta ci vogliono alcuni minuti)...
cargo build --release
if errorlevel 1 (
  echo.
  echo ERRORE: hai installato Rust da https://rustup.rs ?
  pause
  exit /b 1
)
if not exist dist mkdir dist
copy /Y "target\release\gestore-file-cp.exe" "dist\Gestore File CP.exe" >nul
echo.
echo Programma pronto: dist\Gestore File CP.exe
echo.

set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if exist "%ISCC%" (
  echo Creo l'installer...
  "%ISCC%" installer\gestore-file-cp.iss
  if errorlevel 1 ( echo ERRORE nella creazione dell'installer. & pause & exit /b 1 )
  echo.
  echo Installer pronto: dist\Installa Gestore File CP 1.0.exe
) else (
  echo Per creare anche l'installer installa Inno Setup da https://jrsoftware.org/isdl.php
  echo e poi rilancia questo file.
)
pause
