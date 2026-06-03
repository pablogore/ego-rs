# auto-commit.ps1 — Stage and commit all changes after a Spec Kit command.
#
# Usage: .\auto-commit.ps1 <event_name>
# Example: .\auto-commit.ps1 after_specify

param(
    [Parameter(Mandatory=$true)]
    [string]$Event
)

function Write-WarningMsg {
    param([string]$Message)
    Write-Host "[auto-commit] WARNING: $Message" -ForegroundColor Yellow
}

function Write-Skip {
    param([string]$Message)
    Write-Host "[auto-commit] SKIP: $Message"
}

function Write-Done {
    param([string]$Message)
    Write-Host "[auto-commit] DONE: $Message" -ForegroundColor Green
}

# Locate config relative to this script's location
$ScriptPath = $MyInvocation.MyCommand.Path
$ScriptDir = Split-Path -Parent $ScriptPath
$ExtensionDir = Resolve-Path "$ScriptDir\..\..\.."
$ConfigFile = Join-Path $ExtensionDir "git\git-config.yml"

if (-not (Test-Path $ConfigFile)) {
    Write-Skip "config file not found at $ConfigFile"
    exit 0
}

# Parse YAML config (basic parsing without external dependencies)
$ConfigContent = Get-Content $ConfigFile -Raw

# Extract event-specific section
$EventSection = $null
$InEvent = $false
$DefaultEnabled = $false
$DefaultMsg = ""

foreach ($line in (Get-Content $ConfigFile)) {
    $trimmed = $line.Trim()
    if ($trimmed -eq "default:") {
        $InEvent = $true
        $EventSection = "default"
        continue
    }
    if ($trimmed -match "^[a-z_]+\:") {
        $InEvent = ($trimmed -eq "$($Event):")
        if ($InEvent) {
            $EventSection = $Event
            continue
        }
        # Only reset if we weren't tracking the event we care about
        if ($EventSection -eq "default" -or $EventSection -eq $Event) {
            $EventSection = $null
            $InEvent = $false
        }
    }
    if ($InEvent -and $trimmed -match "^enabled:\s*(true|false)") {
        $val = $Matches[1]
        if ($EventSection -eq $Event) {
            $EventEnabled = ($val -eq "true")
        }
        if ($EventSection -eq "default") {
            $DefaultEnabled = ($val -eq "true")
        }
    }
    if ($InEvent -and $trimmed -match "^message:\s*""(.+)""") {
        if ($EventSection -eq $Event) {
            $EventMsg = $Matches[1]
        }
        if ($EventSection -eq "default") {
            $DefaultMsg = $Matches[1]
        }
    }
}

# Determine if enabled
$Enabled = $false
$CommitMsg = ""

if ($EventEnabled -eq $true) {
    $Enabled = $true
    $CommitMsg = $EventMsg
} elseif ($DefaultEnabled -eq $true) {
    $Enabled = $true
    $CommitMsg = $DefaultMsg
}

if (-not $Enabled) {
    Write-Skip "auto-commit not enabled for event '$Event'"
    exit 0
}

# Check for uncommitted changes
$RepoPath = git rev-parse --show-toplevel 2>$null
if (-not $?) {
    Write-WarningMsg "not a git repository"
    exit 0
}

$HasChanges = $false
$Status = git status --porcelain 2>$null
if ($Status) {
    $HasChanges = $true
}

if (-not $HasChanges) {
    Write-Skip "no changes to commit"
    exit 0
}

# Default message fallback
if ([string]::IsNullOrEmpty($CommitMsg)) {
    $CommitMsg = "[Spec Kit] Auto-commit ($Event)"
}

Write-Host "[auto-commit] Staging all changes..."
git add .

Write-Host "[auto-commit] Committing..."
git commit -m $CommitMsg

Write-Done $CommitMsg
