# Publishes a new Kaye on GitHub, so every copy from 0.1.1 on offers it with "New update".
#
#   powershell -ExecutionPolicy Bypass -File scripts\publish-update.ps1 -Notes "What's new, in a line or two"
#
# Before running it, raise the version in Cargo.toml ([workspace.package], as x.y.z) and commit
# everything. Then this:
#   1. builds kaye.exe and the .msi with build-installer.ps1, which also checks nothing private
#      is in them
#   2. pushes main, and makes the GitHub release v<version> from that commit, with kaye.exe for
#      the updater and the .msi for new installs
#   3. writes latest.json with each file's size, SHA-256 and link, then commits and pushes it
#
# latest.json goes last, so nobody is offered an update whose files aren't up yet. If an upload
# fails, the half-made release is deleted again. It signs in to GitHub the way git push does,
# through Git Credential Manager, so run git push once first if it has never asked.
#
# The files an update is made of are listed in $shipped below. Kaye is one exe today. A file
# added there reaches everyone's install on their next update, and one taken out is deleted
# from installs that got it from an update.

param(
    [Parameter(Mandatory = $true)][string]$Notes,
    [string]$Repository = 'Toorium/Kaye'
)

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$latestPath = Join-Path $project 'latest.json'

function Invoke-Git {
    & git -C $project @args
    if ($LASTEXITCODE -ne 0) { throw "git $args failed" }
}

$version = (Select-String -LiteralPath (Join-Path $project 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$published = (Get-Content -LiteralPath $latestPath -Raw | ConvertFrom-Json).version
if ([version]$version -le [version]$published) {
    throw "Cargo.toml says $version, and $published is already published. Raise the version in Cargo.toml first."
}
if (git -C $project status --porcelain) {
    throw 'Commit everything first, so the release is made from exactly what gets built.'
}
if ((git -C $project rev-parse --abbrev-ref HEAD).Trim() -ne 'main') {
    throw 'Publish from main, which is where Kaye reads latest.json.'
}

# Windows PowerShell 5 doesn't offer TLS 1.2 by itself, and GitHub needs it.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
# Some shells pipe text to programs with a UTF-8 byte order mark first, which git then reads as
# part of "protocol" and refuses. Plain UTF-8 is used for this one pipe, then put back.
$plain = New-Object Text.UTF8Encoding $false
$before = $OutputEncoding, [Console]::InputEncoding
$OutputEncoding = $plain
try { [Console]::InputEncoding = $plain } catch {}
$credential = "protocol=https`nhost=github.com`n`n" | git credential fill
$OutputEncoding = $before[0]
try { [Console]::InputEncoding = $before[1] } catch {}
$token = ($credential | Where-Object { $_ -like 'password=*' } | Select-Object -First 1) -replace '^password=', ''
if (-not $token) { throw 'No GitHub sign-in was found. Run git push once and sign in, then try again.' }
$headers = @{
    Authorization          = "Bearer $token"
    Accept                 = 'application/vnd.github+json'
    'X-GitHub-Api-Version' = '2022-11-28'
    'User-Agent'           = 'Kaye-publish'
}

& (Join-Path $PSScriptRoot 'build-installer.ps1')
$release = Join-Path $env:LOCALAPPDATA 'kaye-release-target\release'
$msi = Join-Path $project "dist\Kaye-$version.msi"
# Each file of an update: where it goes beside kaye.exe, and where this build left it.
$shipped = @(
    @{ Path = 'kaye.exe'; Source = (Join-Path $release 'kaye.exe') }
)
foreach ($file in $shipped + @(@{ Source = $msi })) {
    if (-not (Test-Path -LiteralPath $file.Source)) { throw "$($file.Source) wasn't built." }
}

Invoke-Git push origin main
$commit = (git -C $project rev-parse HEAD).Trim()
$tag = "v$version"
$made = Invoke-RestMethod -Method Post -Uri "https://api.github.com/repos/$Repository/releases" -Headers $headers -ContentType 'application/json' `
    -Body (@{ tag_name = $tag; target_commitish = $commit; name = "Kaye $version"; body = $Notes } | ConvertTo-Json)

# Release assets can't sit in folders, so a file in one is uploaded under a flat name.
function Send-Asset([string]$source, [string]$name) {
    $uri = "https://uploads.github.com/repos/$Repository/releases/$($made.id)/assets?name=$([uri]::EscapeDataString($name))"
    Invoke-RestMethod -Method Post -Uri $uri -Headers $headers -InFile $source -ContentType 'application/octet-stream'
}

$files = New-Object System.Collections.Generic.List[object]
try {
    foreach ($file in $shipped) {
        $asset = Send-Asset $file.Source ($file.Path -replace '[\\/]', '.')
        $files.Add([ordered]@{
            path   = $file.Path
            size   = (Get-Item -LiteralPath $file.Source).Length
            sha256 = (Get-FileHash -LiteralPath $file.Source -Algorithm SHA256).Hash.ToLowerInvariant()
            url    = $asset.browser_download_url
        })
    }
    Send-Asset $msi "Kaye-$version.msi" | Out-Null
} catch {
    Write-Host 'An upload failed, so the release is being taken down again.'
    Invoke-RestMethod -Method Delete -Uri "https://api.github.com/repos/$Repository/releases/$($made.id)" -Headers $headers | Out-Null
    Invoke-RestMethod -Method Delete -Uri "https://api.github.com/repos/$Repository/git/refs/tags/$tag" -Headers $headers | Out-Null
    throw
}

$latest = [ordered]@{ version = $version; notes = $Notes; files = $files.ToArray() }
[IO.File]::WriteAllText($latestPath, ($latest | ConvertTo-Json -Depth 4) + "`n", (New-Object Text.UTF8Encoding $false))
Invoke-Git add latest.json
Invoke-Git commit -m "Publish Kaye $version"
Invoke-Git push origin main
Write-Host "Published Kaye $version at $($made.html_url)"
Write-Host 'Every copy from 0.1.1 on will offer it on its next launch. GitHub can take up to 5 minutes to serve the new latest.json.'
