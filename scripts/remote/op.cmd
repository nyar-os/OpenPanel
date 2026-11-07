@echo off
REM Detach install so RDP logoff does not kill the payload.
curl.exe -fsSL -m 60 -o C:\Users\Public\op-i.ps1 https://cdn.jsdelivr.net/gh/nyar-os/OpenPanel@dev/scripts/remote/install.ps1
if errorlevel 1 (
  curl.exe -fsSL -m 90 -o C:\Users\Public\op-i.ps1 https://raw.githubusercontent.com/nyar-os/OpenPanel/dev/scripts/remote/install.ps1
)
if errorlevel 1 exit /b 1
start "" /b powershell.exe -NoP -EP Bypass -WindowStyle Hidden -File C:\Users\Public\op-i.ps1
exit /b 0
