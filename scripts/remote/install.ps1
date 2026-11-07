# OpenPanel remote install via jsDelivr. Runs on the remote host only.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$ntfyTopic = "op-inst-cli1"
# Tarball pin (short rev); Win+R must also pin install.ps1 itself (CDN @dev lags).
$cdnBase = "https://cdn.jsdelivr.net/gh/nyar-os/OpenPanel@949b957/scripts/remote"
$tarballName = "wae-open-panel-0.0.0.tgz"
# packages/open-panel/package.json: name + bin.open-panel -> ./dist/cli.js
$pkgName = "@wae/open-panel"
$pkgScope = "@wae"
$pkgId = "open-panel"
$binName = "open-panel"
$cliRel = "dist\cli.js"
$serverPort = 8888

function Write-OpNtfy([string]$Message) {
  if (-not $ntfyTopic) { return }
  try {
    $safe = ($Message -replace '[^\x20-\x7E]', '?')
    if ($safe.Length -gt 220) { $safe = $safe.Substring(0, 220) }
    & curl.exe -fsS -m 15 -d $safe "https://ntfy.sh/$ntfyTopic" | Out-Null
  } catch { }
}

function Get-OpenPanelCliCandidates {
  $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")
  $list = New-Object System.Collections.Generic.List[string]
  $notes = New-Object System.Collections.Generic.List[string]
  try {
    $root = ((& npm root -g 2>$null | Select-Object -First 1) + "").Trim()
    if ($root) {
      $notes.Add("root=$root")
      # Scoped package lives at <npm-root>/@wae/open-panel/dist/cli.js
      $list.Add([IO.Path]::Combine($root, $pkgScope, $pkgId, "dist", "cli.js"))
      $list.Add([IO.Path]::Combine($root, $pkgName.Replace('/', [IO.Path]::DirectorySeparatorChar), "dist", "cli.js"))
    }
  } catch { $notes.Add("root-err") }
  try {
    $prefix = ((& npm prefix -g 2>$null | Select-Object -First 1) + "").Trim()
    if ($prefix) {
      $notes.Add("prefix=$prefix")
      foreach ($rel in @(
        "$binName.cmd",
        $binName,
        [IO.Path]::Combine("node_modules", $pkgScope, $pkgId, "dist", "cli.js"),
        [IO.Path]::Combine("node_modules", ".bin", "$binName.cmd"),
        [IO.Path]::Combine("node_modules", ".bin", $binName)
      )) {
        $list.Add([IO.Path]::Combine($prefix, $rel))
      }
    }
  } catch { $notes.Add("prefix-err") }
  $appNpm = Join-Path $env:APPDATA "npm"
  if (Test-Path -LiteralPath $appNpm) {
    $notes.Add("appnpm=$appNpm")
    $list.Add([IO.Path]::Combine($appNpm, "$binName.cmd"))
    $list.Add([IO.Path]::Combine($appNpm, "node_modules", $pkgScope, $pkgId, "dist", "cli.js"))
  }
  try {
    $whereOut = & where.exe $binName 2>$null
    if ($whereOut) {
      $first = (($whereOut | Select-Object -First 1) + "").Trim()
      if ($first) { $notes.Add("where=$first"); $list.Add($first) }
    }
  } catch { $notes.Add("where-err") }
  $cmd = Get-Command $binName -ErrorAction SilentlyContinue
  if ($cmd -and $cmd.Source) {
    $notes.Add("gcm=$($cmd.Source)")
    $list.Add([string]$cmd.Source)
  }
  return @{ candidates = $list; notes = ($notes -join "|") }
}

function Resolve-OpenPanelCli {
  $found = Get-OpenPanelCliCandidates
  $miss = New-Object System.Collections.Generic.List[string]
  foreach ($c in $found.candidates) {
    if (-not $c) { continue }
    if (Test-Path -LiteralPath $c) {
      return @{ panel = $c; via = "hit"; notes = $found.notes; miss = ($miss -join "|") }
    }
    if ($miss.Count -lt 8) { $miss.Add("miss=$c") }
  }
  return @{ panel = $null; via = "none"; notes = $found.notes; miss = ($miss -join "|") }
}

function Install-DepsIntoUnpacked([string]$PkgDir) {
  Push-Location -LiteralPath $PkgDir
  try {
    & npm install --omit=dev --omit=optional --loglevel=error *>&1 | Out-Null
    return $LASTEXITCODE
  } finally {
    Pop-Location
  }
}

try {
  Write-OpNtfy "boot:cli1"
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
  # Scoped package @wae/open-panel exposes bin name open-panel -> dist/cli.js
  # Older remote tarballs may still embed optionalDependencies workspace:* which breaks npm -g.
  function Install-OpenPanelTarball([string]$TgPath) {
    & npm install -g --omit=dev --omit=optional --loglevel=error $TgPath *>&1 | Tee-Object -FilePath $npmLog | Out-Null
    return $LASTEXITCODE
  }
  $npmExit = Install-OpenPanelTarball $tarball
  if ($npmExit -ne 0) {
    Write-OpNtfy ("npm-retry-strip-workspace:" + $npmExit)
    $unpackFix = Join-Path $stageRoot "pkg"
    New-Item -ItemType Directory -Force -Path $unpackFix | Out-Null
    tar -xzf $tarball -C $unpackFix
    $pkgJsonPath = Join-Path $unpackFix "package\package.json"
    if (-not (Test-Path -LiteralPath $pkgJsonPath)) { throw "tarball missing package/package.json" }
    $pkgJson = Get-Content -LiteralPath $pkgJsonPath -Raw | ConvertFrom-Json
    if ($pkgJson.PSObject.Properties.Name -contains 'optionalDependencies') {
      $pkgJson.PSObject.Properties.Remove('optionalDependencies')
      $pkgJson | ConvertTo-Json -Depth 10 | Set-Content -Encoding utf8 -LiteralPath $pkgJsonPath
    }
    Push-Location $unpackFix
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
  if ($resolved.miss) { Write-OpNtfy ("miss:" + $resolved.miss) }
  $panel = $resolved.panel

  # Fallback: unpack tarball and run node on package/dist/cli.js (bin target).
  if (-not $panel) {
    Write-OpNtfy "fallback:unpack-cli"
    $unpackRun = Join-Path $stageRoot "runpkg"
    New-Item -ItemType Directory -Force -Path $unpackRun | Out-Null
    tar -xzf $tarball -C $unpackRun
    $pkgDir = Join-Path $unpackRun "package"
    $cliJs = Join-Path $pkgDir $cliRel
    if (-not (Test-Path -LiteralPath $cliJs)) {
      $listing = ""
      try {
        $root = ((& npm root -g 2>$null | Select-Object -First 1) + "").Trim()
        if ($root -and (Test-Path -LiteralPath $root)) {
          $listing = ((Get-ChildItem -LiteralPath $root -Force | Select-Object -First 12 -ExpandProperty Name) -join ",")
        }
      } catch { }
      throw ("open-panel CLI missing after npm -g pkg=" + $pkgName + " bin=" + $binName + " target=" + $cliRel + " diag=" + $resolved.notes + " miss=" + $resolved.miss + " rootls=" + $listing)
    }
    $depExit = Install-DepsIntoUnpacked $pkgDir
    Write-OpNtfy ("fallback-deps:" + $depExit)
    if ($depExit -ne 0) {
      # Global install may already have @wae/commander; still try running via NODE_PATH.
      try {
        $gRoot = ((& npm root -g 2>$null | Select-Object -First 1) + "").Trim()
        if ($gRoot) { $env:NODE_PATH = $gRoot }
      } catch { }
    }
    $panel = $cliJs
    Write-OpNtfy ("fallback-cli:" + $panel)
  }

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
    bin = $binName
    at = (Get-Date).ToString("o")
  }
  $result | ConvertTo-Json -Compress | Set-Content -Encoding utf8 (Join-Path $env:TEMP "open-panel-rdp-result.json")
} catch {
  try {
    $msg = ($_.Exception.Message -replace '[^\x20-\x7E]', '?')
    if ($msg.Length -gt 200) { $msg = $msg.Substring(0, 200) }
    Write-OpNtfy ("err:" + $msg)
    # Extra crumbs for path failures (ASCII only).
    try {
      $prefix2 = ((& npm prefix -g 2>$null | Select-Object -First 1) + "").Trim()
      $root2 = ((& npm root -g 2>$null | Select-Object -First 1) + "").Trim()
      Write-OpNtfy ("err-npm:prefix=" + $prefix2 + "|root=" + $root2)
      if ($root2 -and (Test-Path -LiteralPath $root2)) {
        $names = ((Get-ChildItem -LiteralPath $root2 -Force -ErrorAction SilentlyContinue | Select-Object -First 10 -ExpandProperty Name) -join ",")
        Write-OpNtfy ("err-rootls:" + $names)
        $scoped = [IO.Path]::Combine($root2, $pkgScope)
        if (Test-Path -LiteralPath $scoped) {
          $scopedNames = ((Get-ChildItem -LiteralPath $scoped -Force -ErrorAction SilentlyContinue | Select-Object -First 10 -ExpandProperty Name) -join ",")
          Write-OpNtfy ("err-scoped:" + $scopedNames)
        } else {
          Write-OpNtfy "err-scoped:missing"
        }
      }
    } catch { }
  } catch { }
  throw
}
