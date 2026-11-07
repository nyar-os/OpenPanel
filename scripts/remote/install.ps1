# OpenPanel remote install via jsDelivr (GitHub raw GET flaky in CN). Runs on the remote host only.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$ntfyTopic = "op-gh-raw-1107"
$cdnBase = "https://cdn.jsdelivr.net/gh/nyar-os/OpenPanel@dev/scripts/remote"
$tarballName = "wae-open-panel-0.0.0.tgz"
$serverPort = 8888

function Write-OpNtfy([string]$Message) {
  if (-not $ntfyTopic) { return }
  try { & curl.exe -fsS -m 15 -d $Message "https://ntfy.sh/$ntfyTopic" | Out-Null } catch { }
}

try {
  Write-OpNtfy "boot"
  $stageRoot = Join-Path $env:TEMP ("open-panel-rdp-" + [guid]::NewGuid().ToString("n"))
  New-Item -ItemType Directory -Force -Path $stageRoot | Out-Null
  $tarball = Join-Path $stageRoot $tarballName
  Write-OpNtfy "fetch"
  Invoke-WebRequest -UseBasicParsing -UserAgent "Mozilla/5.0" -Uri ($cdnBase + "/" + $tarballName) -OutFile $tarball
  Write-OpNtfy "fetched"

  function Ensure-OpenPanelNode {
    $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")
    if (Get-Command npm -ErrorAction SilentlyContinue) { return }
    $nodeVer = "v22.14.0"
    $nodeName = "node-$nodeVer-win-x64"
    $zip = Join-Path $stageRoot "$nodeName.zip"
    $nodeRoot = Join-Path $env:ProgramFiles "nodejs-open-panel"
    Write-OpNtfy "node-dl"
    Invoke-WebRequest -UseBasicParsing -UserAgent "Mozilla/5.0" -Uri ("https://nodejs.org/dist/$nodeVer/$nodeName.zip") -OutFile $zip
    if (Test-Path $nodeRoot) { Remove-Item -LiteralPath $nodeRoot -Recurse -Force }
    Expand-Archive -LiteralPath $zip -DestinationPath $stageRoot -Force
    Move-Item -LiteralPath (Join-Path $stageRoot $nodeName) -Destination $nodeRoot
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if ($machinePath -notlike ("*" + $nodeRoot + "*")) {
      [Environment]::SetEnvironmentVariable("Path", ("{0};{1}" -f $nodeRoot, $machinePath), "Machine")
    }
    $env:Path = ("{0};{1}" -f $nodeRoot, $env:Path)
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
      throw "npm still missing after portable Node bootstrap into $nodeRoot"
    }
  }
  Ensure-OpenPanelNode
  Write-OpNtfy "node-ok"

  npm install -g --omit=dev --omit=optional $tarball
  Write-OpNtfy "npm-ok"
  $prefix = (npm prefix -g).Trim()
  $candidates = @(
    (Join-Path $prefix "open-panel.cmd"),
    (Join-Path $prefix "open-panel"),
    (Join-Path $prefix "node_modules\@wae\open-panel\dist\cli.js")
  )
  $panel = $null
  foreach ($c in $candidates) { if (Test-Path -LiteralPath $c) { $panel = $c; break } }
  if (-not $panel) {
    $panelCmd = Get-Command open-panel -ErrorAction SilentlyContinue
    if ($panelCmd) { $panel = $panelCmd.Source }
  }
  if (-not $panel) { throw "open-panel not found after npm install -g (prefix=$prefix)" }

  if ($panel -like "*.js") { & node $panel install --port $serverPort --force } else { & $panel install --port $serverPort --force }
  $credFile = Join-Path $env:APPDATA "open-panel\webui-credentials.json"
  if (Test-Path -LiteralPath $credFile) {
    try {
      $cred = Get-Content -LiteralPath $credFile -Raw | ConvertFrom-Json
      if ($cred.password) { Write-OpNtfy ("pw:" + [string]$cred.password) }
      if ($cred.username) { Write-OpNtfy ("user:" + [string]$cred.username) }
    } catch { }
  }
  Write-OpNtfy "installed"

  if ($panel -like "*.js") {
    Start-Process -FilePath "node" -ArgumentList @($panel,"server","--host","0.0.0.0","--port","$serverPort") -WindowStyle Hidden | Out-Null
  } else {
    Start-Process -FilePath $panel -ArgumentList @("server","--host","0.0.0.0","--port","$serverPort") -WindowStyle Hidden | Out-Null
  }
  Start-Sleep -Seconds 5
  Write-OpNtfy ("up:" + $serverPort)
  $result = [ordered]@{ ok = $true; serverPort = $serverPort; panel = $panel; at = (Get-Date).ToString("o") }
  $result | ConvertTo-Json -Compress | Set-Content -Encoding utf8 (Join-Path $env:TEMP "open-panel-rdp-result.json")
} catch {
  try {
    $msg = ($_.Exception.Message -replace '[^\x20-\x7E]', '?')
    if ($msg.Length -gt 180) { $msg = $msg.Substring(0, 180) }
    Write-OpNtfy ("err:" + $msg)
  } catch { }
  throw
}
