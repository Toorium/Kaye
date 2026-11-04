# Builds the file to hand out: dist\Kaye-<version>.msi
#
# Run it from anywhere:
#   powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
#
# This build differs from `cargo build --release` in two ways, which is why it has a target
# folder of its own and doesn't slow down everyday builds:
#   - The C runtime goes inside the exe, so a computer with no Visual C++ redistributable
#     still runs it.
#   - This computer's folder names are taken out of the exe.
#
# Nothing here reads settings into the build. Kaye's settings and API key live in
# %APPDATA%\Kaye and only ever get there at runtime. The last step proves it, and stops the
# build if anything from this computer is in the file.

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $project 'dist'
$target = Join-Path $env:LOCALAPPDATA 'kaye-release-target'
$release = Join-Path $target 'release'

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $args failed" }
}

$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
# The last match wins, so the most specific go last. A folder can be a link to somewhere else,
# and then the real path is the one that ends up in the exe.
$hidden = New-Object System.Collections.Generic.List[string]
$hidden.Add($env:USERPROFILE)
foreach ($folder in @($cargoHome, $rustupHome, $project)) {
    $hidden.Add($folder)
    $real = (Get-Item -LiteralPath $folder -Force).Target
    if ($real) { $hidden.Add([string]@($real)[0]) }
}
$flags = @('-C', 'target-feature=+crt-static')
foreach ($folder in $hidden) { $flags += "--remap-path-prefix=$folder=build" }
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f
# Luau is C++, which Rust's flags don't reach, and it keeps its source paths in __FILE__.
$env:CXXFLAGS = "/d1trimfile:$cargoHome\"
$env:CFLAGS = $env:CXXFLAGS
$env:CARGO_TARGET_DIR = $target
$env:CARGO_PROFILE_RELEASE_STRIP = 'symbols'

Push-Location $project
try {
    Invoke-Cargo build --release --locked -p kaye --bin kaye
} finally {
    Pop-Location
}

# The package is a Windows Installer .msi, made by WiX 3 from installer\msi\kaye.wxs. WiX
# isn't part of Rust, so it's looked for in WIX, then %USERPROFILE%\tools\wix314, where
# https://github.com/wixtoolset/wix3/releases (wix314-binaries.zip) was unpacked.
$wix = @($env:WIX, (Join-Path $env:USERPROFILE 'tools\wix314')) | Where-Object { $_ } |
    ForEach-Object { if (Test-Path (Join-Path $_ 'candle.exe')) { $_ } elseif (Test-Path (Join-Path $_ 'bin\candle.exe')) { Join-Path $_ 'bin' } } |
    Select-Object -First 1
if (-not $wix) { throw 'WiX 3 was not found. Unpack wix314-binaries.zip to %USERPROFILE%\tools\wix314.' }

$version = (Select-String -LiteralPath (Join-Path $project 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$msiSource = Join-Path $project 'installer\msi'
$work = Join-Path $target 'msi'
New-Item -ItemType Directory -Force $work, $dist | Out-Null

# The wizard's pictures, drawn from the logo: Kaye's coral down the side of the first and
# last pages, and the mark at the right of the banner on the others.
Add-Type -AssemblyName System.Drawing
$mark = [System.Drawing.Image]::FromFile((Join-Path $project 'assets\logo_mark.png'))
$coral = [System.Drawing.Color]::FromArgb(0xF3, 0x6B, 0x70)
function Save-Picture([int]$width, [int]$height, [string]$path, [scriptblock]$draw) {
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.Clear([System.Drawing.Color]::White)
    & $draw $g
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $g.Dispose(); $bitmap.Dispose()
}
$dialog = Join-Path $work 'dialog.bmp'
$banner = Join-Path $work 'banner.bmp'
Save-Picture 493 312 $dialog { param($g)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush $coral), 0, 0, 164, 312)
    $g.DrawImage($mark, 42, 40, 80, 80)
}
Save-Picture 493 58 $banner { param($g)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush $coral), 437, 7, 44, 44)
    $g.DrawImage($mark, 443, 13, 32, 32)
}
$mark.Dispose()

$installer = Join-Path $dist "Kaye-$version.msi"
$object = Join-Path $work 'kaye.wixobj'
& (Join-Path $wix 'candle.exe') -nologo -arch x64 -ext WixUtilExtension -out $object `
    "-dVersion=$version" "-dProgram=$(Join-Path $release 'kaye.exe')" "-dIcon=$(Join-Path $project 'assets\kaye.ico')" `
    "-dNotice=$(Join-Path $msiSource 'notice.rtf')" "-dBanner=$banner" "-dDialog=$dialog" (Join-Path $msiSource 'kaye.wxs')
if ($LASTEXITCODE -ne 0) { throw 'candle failed' }
# ICE91 only warns that per-user files won't follow a per-machine install, which this never is.
# ICE61 warns that a package can replace its own version, which test builds rebuilt at the same
# version need.
& (Join-Path $wix 'light.exe') -nologo -ext WixUIExtension -ext WixUtilExtension -cultures:en-us -sice:ICE91 -sice:ICE61 -spdb -out $installer $object
if ($LASTEXITCODE -ne 0) { throw 'light failed' }

# The installer's copy of Kaye is compressed, so the programs are searched before packing.
$latin = [Text.Encoding]::GetEncoding(28591)
function Find-Text([byte[]]$bytes, [string]$wanted) {
    $text = $latin.GetString($bytes)
    $wide = $latin.GetString([Text.Encoding]::Unicode.GetBytes($wanted))
    return $text.IndexOf($wanted, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or $text.IndexOf($wide, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

$private = New-Object System.Collections.Generic.List[string]
$private.Add("\$env:USERNAME\")
$private.Add("/$env:USERNAME/")
foreach ($folder in $hidden) {
    $private.Add($folder)
    $private.Add($folder.Replace('\', '/'))
}
$settings = Join-Path $env:APPDATA 'Kaye\settings.json'
if (Test-Path -LiteralPath $settings) {
    $key = (Get-Content -LiteralPath $settings -Raw | ConvertFrom-Json).system.roblox_api_key
    if ($key) { $private.Add($key) }
}
# Kaye now keeps the key in Credential Manager (src/secrets.rs), so look there too.
Add-Type -Namespace KayeBuild -Name Credentials -MemberDefinition @'
[DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
static extern bool CredReadW(string target, int type, int flags, out IntPtr credential);
[DllImport("advapi32.dll")]
static extern void CredFree(IntPtr buffer);
public static string Read(string target) {
    IntPtr found;
    if (!CredReadW(target, 1, 0, out found)) { return null; }
    try {
        int size = Marshal.ReadInt32(found, IntPtr.Size == 8 ? 32 : 24);
        IntPtr blob = Marshal.ReadIntPtr(found, IntPtr.Size == 8 ? 40 : 28);
        byte[] bytes = new byte[size];
        Marshal.Copy(blob, bytes, 0, size);
        return System.Text.Encoding.UTF8.GetString(bytes);
    } finally { CredFree(found); }
}
'@
$stored = [KayeBuild.Credentials]::Read('Kaye/Roblox API key')
if ($stored) { $private.Add($stored) }

$leaks = 0
foreach ($program in (Join-Path $release 'kaye.exe'), $installer) {
    $bytes = [IO.File]::ReadAllBytes($program)
    for ($index = 0; $index -lt $private.Count; $index++) {
        # Saying which by number keeps a key out of the build log.
        if (Find-Text $bytes $private[$index]) {
            Write-Host "$program contains private text number $index of $($private.Count)"
            $leaks++
        }
    }
}
if ($leaks -gt 0) {
    Remove-Item -LiteralPath $installer
    throw 'Something from this computer is in the build, so the installer was deleted.'
}

$size = [math]::Round((Get-Item -LiteralPath $installer).Length / 1MB, 1)
Write-Host "Checked $($private.Count) private strings in both programs and found none."
Write-Host "Built $installer ($size MB)"
