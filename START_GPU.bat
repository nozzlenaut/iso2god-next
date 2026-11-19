@echo off
setlocal
set "ISO2GOD_GPU=1"
set "ISO2GOD_GPU_LOG=%TEMP%\ISO2GOD-GPU.log"
> "%ISO2GOD_GPU_LOG%" echo GPU mode requested. The selected device will appear after conversion begins.
echo GPU status log: %ISO2GOD_GPU_LOG%
cd /d "%~dp0"
if not exist "iso2god-next-gui.exe" (
  echo Put this file next to iso2god-next-gui.exe and try again.
  pause
  exit /b 1
)
start "" /wait "%~dp0iso2god-next-gui.exe" %*
type "%ISO2GOD_GPU_LOG%"
pause
