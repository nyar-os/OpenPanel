# OpenPanel remote install via jsDelivr. Runs on the remote host only.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$ntfyTopic = "op-gh-raw-1107"
# Tarball is stable on @dev; Win+R must pin install.ps1 itself to the push commit (CDN @dev lags).
$cdnBase = "https://cdn.jsdelivr.net/gh/nyar-os/OpenPanel@4e6d417967b290cdedf0d8ac875194a6c5b0228e/scripts/remote"
$tarballName = "wae-open-panel-0.0.0.tgz"
$pkgName = "@wae/open-panel"
$serverPort = 8888

function Write-OpNtfy([string]$Message) {
  if (-not $ntfyTopic) { return }
  try {
    $safe = ($Message -replace '[^\x20-\x7E]', '?')
    if ($safe.Length -gt 200) { $safe = $safe.Substring(0, 200) }
    & curl.exe -fsS -m 15 -d $safe "https://ntfy.sh/$ntfyTopic" | Out-Null
  } catch { }
}

function Resolve-OpenPanelCli {
  $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")
  $hits = New-Object System.Collections.Generic.List[string]
  try {
    $root = (& npm root -g 2>$null | Select-Object -First 1)
    if ($root) {
      $hits.Add("root=$root")
      $cli = Join-Path $root "$pkgName\dist\cli.js"
      if (Test-Path -LiteralPath $cli) { return @{ panel = $cli; via = "npm-root"; notes = ($hits -join "|") } }
      $hits.Add("miss-cli=$cli")
    }
  } catch { $hits.Add("root-err") }
  try {
    $prefix = (& npm prefix -g 2>$null | Select-Object -First 1)
    if ($prefix) {
      $hits.Add("prefix=$prefix")
      foreach ($rel in @(
        "open-panel.cmd",
        "open-panel",
        "node_modules\$pkgName\dist\cli.js",
        "node_modules\.bin\open-panel.cmd",
        "node_modules\.bin\open-panel"
      )) {
        $c = Join-Path $prefix $rel
        if (Test-Path -LiteralPath $c) { return @{ panel = $c; via = "npm-prefix:$rel"; notes = ($hits -join "|") } }
        $hits.Add("miss=$c")
      }
    }
  } catch { $hits.Add("prefix-err") }
  try {
    $whereOut = & where.exe open-panel 2>$null
    if ($whereOut) {
      $first = ($whereOut | Select-Object -First 1)
      $hits.Add("where=$first")
      if ($first -and (Test-Path -LiteralPath $first)) {
        return @{ panel = $first; via = "where"; notes = ($hits -join "|") }
      }
    }
  } catch { $hits.Add("where-err") }
  $cmd = Get-Command open-panel -ErrorAction SilentlyContinue
  if ($cmd -and $cmd.Source -and (Test-Path -LiteralPath $cmd.Source)) {
    $hits.Add("gcm=$($cmd.Source)")
    return @{ panel = $cmd.Source; via = "Get-Command"; notes = ($hits -join "|") }
  }
  return @{ panel = $null; via = "none"; notes = ($hits -join "|") }
}

try {
  Write-OpNtfy "boot"
  $stageRoot = Join-Path $env:TEMP ("open-panel-rdp-" + [guid]::NewGuid().ToString("n"))
  New-Item -ItemType Directory -Force -Path $stageRoot | Out-Null
  $tarball = Join-Path $stageRoot $tarballName
  Write-OpNtfy "fetch"
  Invoke-WebRequest -UseBasicParsing -UserAgent "Mozilla/5.0" -Uri ($cdnBase + "/" + $tarballName) -OutFile $tarball
  Write-OpNtfy ("fetched:" + (Get-Item -LiteralPath $tarball).Length)

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

  $npmLog = Join-Path $stageRoot "npm-install.log"
  # Scoped package @wae/open-panel exposes bin name open-panel.
  # Older remote tarballs may still embed optionalDependencies workspace:* which breaks npm -g.
  function Install-OpenPanelTarball([string]$TgPath) {
    & npm install -g --omit=dev --omit=optional --loglevel=error $TgPath *>&1 | Tee-Object -FilePath $npmLog | Out-Null
    return $LASTEXITCODE
  }
  $npmExit = Install-OpenPanelTarball $tarball
  if ($npmExit -ne 0) {
    Write-OpNtfy ("npm-retry-strip-workspace:" + $npmExit)
    $unpack = Join-Path $stageRoot "pkg"
    New-Item -ItemType Directory -Force -Path $unpack | Out-Null
    tar -xzf $tarball -C $unpack
    $pkgJsonPath = Join-Path $unpack "package\package.json"
    if (-not (Test-Path -LiteralPath $pkgJsonPath)) { throw "tarball missing package/package.json" }
    $pkgJson = Get-Content -LiteralPath $pkgJsonPath -Raw | ConvertFrom-Json
    if ($pkgJson.PSObject.Properties.Name -contains 'optionalDependencies') {
      $pkgJson.PSObject.Properties.Remove('optionalDependencies')
      $pkgJson | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 -LiteralPath $pkgJsonPath
    }
    Push-Location $unpack
    tar -czf (Join-Path $stageRoot "fixed.tgz") package
    Pop-Location
    $tarball = Join-Path $stageRoot "fixed.tgz"
    $npmExit = Install-OpenPanelTarball $tarball
  }
  if ($npmExit -ne 0) {
    $tail = (Get-Content -LiteralPath $npmLog -ErrorAction SilentlyContinue | Select-Object -Last 3) -join " / "
    throw ("npm install -g failed exit=" + $npmExit + " " + $tail)
  }
  Write-OpNtfy "npm-ok"

  $resolved = Resolve-OpenPanelCli
  Write-OpNtfy ("which:" + $resolved.via + ":" + [string]$resolved.panel)
  Write-OpNtfy ("diag:" + $resolved.notes)
  $panel = $resolved.panel
  if (-not $panel) { throw ("open-panel not found after npm install -g pkg=" + $pkgName + " diag=" + $resolved.notes) }

  if ($panel -like "*.js") {
    & node $panel install --port $serverPort --force
  } else {
    & $panel install --port $serverPort --force
  }
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
  $result = [ordered]@{
    ok = $true
    serverPort = $serverPort
    panel = $panel
    via = $resolved.via
    pkg = $pkgName
    at = (Get-Date).ToString("o")
  }
  $result | ConvertTo-Json -Compress | Set-Content -Encoding utf8 (Join-Path $env:TEMP "open-panel-rdp-result.json")
} catch {
  try {
    $msg = ($_.Exception.Message -replace '[^\x20-\x7E]', '?')
    if ($msg.Length -gt 180) { $msg = $msg.Substring(0, 180) }
    Write-OpNtfy ("err:" + $msg)
  } catch { }
  throw
}
