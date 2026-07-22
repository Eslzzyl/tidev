---
name: powershell
description: PowerShell syntax guide, common cmdlets, and version-specific pitfalls (5.1 vs 7+). Use when running shell commands on Windows to write correct PowerShell commands.
---

# PowerShell Guide

PowerShell is the default shell on Windows. This guide covers both
**Windows PowerShell 5.1** (`powershell.exe`) and **PowerShell 7+** (`pwsh`).

## Detecting Your Edition

Check your current shell from the `<env>` block in the system prompt:

```
Shell: Windows PowerShell 5.1 (powershell)   → 5.1
Shell: PowerShell 7+ (C:\...\pwsh.exe)       → 7+
Shell: PowerShell 7+ (pwsh)                  → 7+
```

You can also detect at runtime:

```powershell
$PSVersionTable.PSVersion.Major    # → 5 or 7
```

---

## Common Unix → PowerShell Equivalents

| Unix command | PowerShell cmdlet | Aliases |
|---|---|---|
| `ls` | `Get-ChildItem` | `ls`, `dir`, `gci` |
| `cd` | `Set-Location` | `cd`, `sl` |
| `pwd` | `Get-Location` | `pwd`, `gl` |
| `cat` | `Get-Content` | `cat`, `gc`, `type` |
| `echo` | `Write-Output` | `echo`, `write` |
| `cp` | `Copy-Item` | `cp`, `copy`, `ci` |
| `mv` | `Move-Item` | `mv`, `move`, `mi` |
| `rm` | `Remove-Item` | `rm`, `del`, `ri` |
| `mkdir` | `New-Item -ItemType Directory` | `md`, `mkdir` |
| `grep` | `Select-String` | `sls` |
| `find` | `Get-ChildItem -Recurse -Filter` | — |
| `head` | `Get-Content -TotalCount N` | — |
| `tail` | `Get-Content -Tail N` | — |
| `wc` | `Measure-Object -Line -Word -Character` | — |
| `sort` | `Sort-Object` | `sort` |
| `ps` | `Get-Process` | `ps`, `gps` |
| `kill` | `Stop-Process` | `kill`, `spps` |
| `curl` | `Invoke-WebRequest` | `curl`, `iwr` (aliases exist but flags differ) |
| `wget` | `Invoke-WebRequest` | `wget` (alias only, flags differ) |

**⚠️  IMPORTANT**: PowerShell aliases like `ls`, `cat`, `cp`, `rm`, `curl` etc.
are **NOT the same as the Unix commands**. They are thin wrappers around
PowerShell cmdlets and **do NOT accept Unix flags**. For example:

```powershell
# WRONG — Unix flags not supported:
ls -la
cat -n file.txt
curl -s https://example.com

# RIGHT — use PowerShell parameters:
Get-ChildItem -Force
Get-Content file.txt | Select-Object -Skip 1  # skip header lines
Invoke-WebRequest -Uri https://example.com -ErrorAction SilentlyContinue
```

---

## PowerShell Syntax Basics

### Variables

```powershell
$name = "World"
$count = 42
$items = @(1, 2, 3)         # array
$hash = @{ key = "value" }  # hashtable
```

### String Interpolation

```powershell
"Hello $name"                    # → "Hello World"
"Result: $($count + 1)"          # → "Result: 43"
```

### Escape Character

PowerShell uses **backtick** `` ` ``, not backslash:

```powershell
`n    # newline
`t    # tab
`r    # carriage return
``    # literal backtick
$var  # no escape needed for $ in single quotes
```

### Comments

```powershell
# Single line comment
<#
  Multi-line comment
#>
```

### Object Pipeline

PowerShell pipes **objects**, not text:

```powershell
Get-Process | Where-Object { $_.CPU -gt 100 } | Sort-Object CPU -Descending
Get-ChildItem *.log | Select-String -Pattern "ERROR" | Select-Object -First 10
```

Common pipeline cmdlets:

| Cmdlet | Purpose | Like Unix |
|---|---|---|
| `Where-Object { ... }` | Filter objects | `grep` / `awk` |
| `Select-Object -First N` | Take first N | `head -n N` |
| `Select-Object -Last N` | Take last N | `tail -n N` |
| `Select-Object -Skip N` | Skip first N | `tail -n +N+1` |
| `Sort-Object -Property` | Sort by field | `sort -k` |
| `ForEach-Object { ... }` | Transform each | `awk` / `xargs` |
| `Group-Object` | Group by field | `uniq -c` |
| `Measure-Object` | Count/sum/avg | `wc` / `awk` |

### Property Selection & Formatting

```powershell
# Select specific properties (like cut -f)
Get-ChildItem | Select-Object Name, Length, LastWriteTime

# Format as table/list
Get-Process | Format-Table -Property Name, CPU, PM -AutoSize

# Calculated properties
Get-ChildItem | Select-Object Name, @{n='SizeKB'; e={[math]::Round($_.Length/1KB, 1)}}
```

---

## Version-Specific Differences

### Windows PowerShell 5.1 (`powershell.exe`)

These features are **NOT available** and cause parser errors:

| Feature | Example | 5.1 | 7+ |
|---|---|---|---|
| Pipeline chain operators | `cmd1 && cmd2` | ❌ Parser error | ✅ Works like bash |
| Pipeline chain operators | `cmd1 \|\| cmd2` | ❌ Parser error | ✅ Works like bash |
| Ternary operator | `$cond ? $a : $b` | ❌ Parser error | ✅ Available |
| Null-coalescing | `$a ?? $b` | ❌ Parser error | ✅ Available |
| Null-conditional | `$obj?.Prop` | ❌ Parser error | ✅ Available |
| Default encoding | `Out-File` / `Set-Content` | UTF-16 LE with BOM | UTF-8 without BOM |

**5.1-safe alternatives:**

```powershell
# Instead of cmd1 && cmd2:
cmd1; if ($?) { cmd2 }

# Instead of ternary:
if ($cond) { $a } else { $b }

# Ensure UTF-8 output when writing files (5.1):
$content | Out-File -FilePath out.txt -Encoding utf8
$content | Set-Content -Path out.txt -Encoding utf8
```

**⚠️  stderr redirection in 5.1:**

```powershell
# WRONG in 5.1 — wraps stderr in ErrorRecord, sets $? to $false:
grep foo file.txt 2>&1

# RIGHT — stderr is captured automatically, don't redirect it.
# Use $LASTEXITCODE to check exit code:
grep foo file.txt
$LASTEXITCODE
```

### PowerShell 7+ (`pwsh`)

Modern PowerShell with full operator support:

```powershell
# Chaining operators work like bash:
dotnet build && dotnet test
npm install || echo "Install failed"

# Ternary and null-coalescing:
$result = $input ? "yes" : "no"
$value = $nullable ?? "default"

# UTF-8 is default — no special encoding flags needed.
```

---

## Common Tasks

### File Reading

```powershell
# Read entire file
Get-Content file.txt

# Read first/last N lines
Get-Content file.txt -TotalCount 10     # head -n 10
Get-Content file.txt -Tail 10            # tail -n 10

# Read with encoding
Get-Content file.txt -Encoding utf8
```

### File Writing

```powershell
# Write text (⚠️  use -Encoding utf8 on 5.1!)
"Hello World" | Out-File out.txt -Encoding utf8
"Line 1`nLine 2" | Set-Content out.txt -Encoding utf8

# Append
"New line" | Add-Content out.txt -Encoding utf8
```

### Finding Files

```powershell
# Find by name
Get-ChildItem -Recurse -Filter "*.rs"

# Find by name pattern
Get-ChildItem -Recurse -Include "*.rs", "*.toml"

# Find directories only
Get-ChildItem -Directory -Recurse

# Find files matching a pattern in content
Get-ChildItem -Recurse -Filter "*.rs" | Select-String -Pattern "unsafe"
```

### Process Management

```powershell
# List processes
Get-Process
Get-Process -Name "code"*

# Kill process
Stop-Process -Name "notepad" -Force
Stop-Process -Id 1234
```

### Error Handling

```powershell
# Check exit code of native commands
npm install
$LASTEXITCODE        # 0 = success

# Try/catch for cmdlet errors
try {
    Get-Content missing.txt -ErrorAction Stop
} catch [System.IO.FileNotFoundException] {
    Write-Output "File not found"
}

# Global error preference
$ErrorActionPreference = "Stop"    # all cmdlet errors become terminating
```

---

## File Paths on Windows

```powershell
# Backslashes (use single quotes to avoid escape issues)
Get-ChildItem 'C:\Program Files\PowerShell\'

# Forward slashes also work
Get-ChildItem 'C:/Program Files/PowerShell/'

# Use Join-Path for safe path construction
Join-Path 'C:\Users' 'Documents'
```

---

## When to Use This Skill

Load this skill when:
- You see `Shell: Windows PowerShell 5.1` or `Shell: PowerShell 7+` in the env info
- You need to run shell commands on Windows
- A PowerShell command failed due to syntax errors or encoding issues
- You need to translate a Unix command to PowerShell
