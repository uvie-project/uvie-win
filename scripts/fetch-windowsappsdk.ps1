# Downloads the Windows App SDK NuGet and extracts the .winmd metadata files
# (plus the unpackaged bootstrapper DLL) into Frameworks/WindowsAppSDK.
# Mirrors uvie-mac's scripts/fetch-uvie.sh / fetch-sparkle.sh convention:
# dependencies are fetched at dev time, never committed.
#
# Usage: pwsh scripts/fetch-windowsappsdk.ps1 [-Version 1.6.250108002]
param(
    [string]$Version = "1.6.250108002"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$dest = Join-Path $root "Frameworks\WindowsAppSDK"
$nupkg = Join-Path $env:TEMP "wasdk-$Version.nupkg"
$extract = Join-Path $env:TEMP "wasdk-$Version"

Write-Host "Downloading Microsoft.WindowsAppSDK $Version ..."
Invoke-WebRequest -Uri "https://www.nuget.org/api/v2/package/Microsoft.WindowsAppSDK/$Version" -OutFile $nupkg

Expand-Archive -Path $nupkg -DestinationPath $extract -Force
New-Item -ItemType Directory -Force -Path $dest | Out-Null

# winmd metadata windows-bindgen reads.
$winmds = @(
    "lib\uap10.0\Microsoft.UI.Xaml.winmd",
    "lib\uap10.0\Microsoft.UI.Text.winmd",
    "lib\uap10.0\Microsoft.Windows.AppLifecycle.winmd",
    "lib\uap10.0\Microsoft.Windows.ApplicationModel.DynamicDependency.winmd",
    "lib\uap10.0\Microsoft.Windows.ApplicationModel.Resources.winmd",
    "lib\uap10.0.18362\Microsoft.Foundation.winmd",
    "lib\uap10.0.18362\Microsoft.Graphics.winmd",
    "lib\uap10.0.18362\Microsoft.UI.winmd"
)
foreach ($rel in $winmds) {
    Copy-Item (Join-Path $extract $rel) -Destination $dest -Force
}

# Unpackaged-app bootstrapper, resolved by uvie-winui::bootstrap at runtime.
Copy-Item (Join-Path $extract "runtimes\win-x64\native\Microsoft.WindowsAppRuntime.Bootstrap.dll") -Destination $dest -Force

# Microsoft.UI.Xaml.Controls.WebView2 references the WebView2 Core metadata,
# which ships in its own NuGet.
$wvVersion = "1.0.2957.106"
$wvNupkg = Join-Path $env:TEMP "webview2-$wvVersion.nupkg"
$wvExtract = Join-Path $env:TEMP "webview2-$wvVersion"
Write-Host "Downloading Microsoft.Web.WebView2 $wvVersion ..."
Invoke-WebRequest -Uri "https://www.nuget.org/api/v2/package/Microsoft.Web.WebView2/$wvVersion" -OutFile $wvNupkg
Expand-Archive -Path $wvNupkg -DestinationPath $wvExtract -Force
Copy-Item (Join-Path $wvExtract "lib\Microsoft.Web.WebView2.Core.winmd") -Destination $dest -Force

Write-Host "Windows App SDK metadata ready under $dest"
