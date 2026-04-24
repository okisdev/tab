pub const SCRIPT: &str = r#"
# tab - terminal autocomplete plugin (PowerShell / pwsh)
# Install: tab init pwsh | Out-String | Invoke-Expression
# Persist (paste into $PROFILE):
#   tab init pwsh | Out-String | Invoke-Expression

if (-not (Get-Module -ListAvailable PSReadLine)) {
    Write-Warning "tab: PSReadLine module is required."
    return
}
Import-Module PSReadLine -ErrorAction SilentlyContinue

Set-PSReadLineKeyHandler -Key Tab -ScriptBlock {
    param($key, $arg)
    $line = $null
    $cursor = $null
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)

    # Resolve the binary on every call so installs/reinstalls during a live
    # session are picked up.
    $tabBin = (Get-Command tab -ErrorAction SilentlyContinue).Source
    if (-not $tabBin) { $tabBin = 'tab' }

    try {
        $selected = & $tabBin complete --buffer "$line" --cwd $PWD.Path 2>$null
    } catch {
        $selected = $null
        $global:LASTEXITCODE = 1
    }

    if ($LASTEXITCODE -eq 0 -and $selected) {
        $selected = ($selected -join [Environment]::NewLine).TrimEnd([char[]]@("`r","`n"))
        [Microsoft.PowerShell.PSConsoleReadLine]::Replace(0, $line.Length, $selected)
        [Microsoft.PowerShell.PSConsoleReadLine]::EndOfLine()
    } else {
        [Microsoft.PowerShell.PSConsoleReadLine]::TabCompleteNext()
    }
}
"#;
