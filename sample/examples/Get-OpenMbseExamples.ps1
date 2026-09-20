# Fetches open-licensed MBSE example models straight from upstream. PowerShell 7, needs git.
# Usage: pwsh ./Get-OpenMbseExamples.ps1 [-Dest C:\path] [-SkipLarge]
param([string]$Dest = "$PWD\open-mbse-examples", [switch]$SkipLarge)
$ErrorActionPreference = 'Stop'

function Get-Repo($repo, $path, $sparse) {
    $target = Join-Path $Dest $path
    if (Test-Path $target) { Write-Host "skip  $path (exists)"; return }
    Write-Host "clone $repo -> $path"
    if ($sparse) {
        git clone --depth 1 --filter=blob:none --sparse -q "https://github.com/$repo.git" $target
        git -C $target sparse-checkout set @sparse
    } else {
        git clone --depth 1 -q "https://github.com/$repo.git" $target
    }
}

New-Item -ItemType Directory -Force $Dest | Out-Null

# SysML v2
Get-Repo 'Systems-Modeling/SysML-v2-Release'   'sysml-v2\omg-sysml-v2-release' @('sysml/src','kerml/src')   # EPL-2.0
Get-Repo 'GfSE/SysML-v2-Models'                'sysml-v2\gfse-models'                                       # BSD-3-Clause
Get-Repo 'MBSE4U/dont-panic-batmobile'         'sysml-v2\mbse4u-batmobile'                                  # Apache-2.0
Get-Repo 'airbus/apollo-11-sysml-v2'           'sysml-v2\airbus-apollo-11'                                  # MPL-2.0
# Arcadia / Capella
Get-Repo 'eclipse-capella/capella'             'arcadia-capella\eclipse-capella-samples' @('samples')       # EPL-2.0
Get-Repo 'dbinfrago/Capella-IFE-sample'        'arcadia-capella\dbinfrago-ife-variant'                      # EPL-2.0 model
# SysML v1
Get-Repo 'gaphor/gaphor'                       'sysml-v1\gaphor-examples' @('examples','docs')              # Apache-2.0

if (-not $SkipLarge) {
    Get-Repo 'MBSE4U/the-sysmlv2-book-examples' 'sysml-v2\mbse4u-sysmlv2-book'                              # Apache-2.0, ~90 MB (Cameo .mdszip)
    Get-Repo 'Open-MBEE/TMT-SysML-Model'        'sysml-v1\openmbee-tmt'                                     # Apache-2.0, ~350 MB incl. slides
}
Write-Host "Done: $Dest"
