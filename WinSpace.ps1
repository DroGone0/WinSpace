[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Path = "$($env:SystemDrive)\\",

    [ValidateRange(1, 8)]
    [int]$Depth = 3,

    [ValidateRange(1, 20)]
    [int]$Top = 8,

    [ValidateSet('Gauge', '3D', 'Both')]
    [string]$VisualMode = 'Both',

    [switch]$IncludeFiles
)

$script:ScanErrors = New-Object System.Collections.Generic.List[object]

function Add-ScanError {
    param(
        [string]$TargetPath,
        [string]$Operation,
        [System.Exception]$Exception
    )

    $script:ScanErrors.Add([pscustomobject]@{
        Path      = $TargetPath
        Operation = $Operation
        Message   = $Exception.Message
        Type      = $Exception.GetType().Name
    })
}

function New-ScanNode {
    param(
        [string]$TargetPath,
        [string]$Name,
        [bool]$IsDirectory,
        [int]$Level,
        [int64]$Size = 0,
        [bool]$Inaccessible = $false
    )

    [pscustomobject]@{
        Path         = $TargetPath
        Name         = $Name
        IsDirectory  = $IsDirectory
        Level        = $Level
        Size         = $Size
        Inaccessible = $Inaccessible
        Children     = New-Object System.Collections.Generic.List[object]
    }
}

function Format-Size {
    param([int64]$Bytes)

    if ($Bytes -lt 0) { $Bytes = 0 }

    $units = @('B', 'KB', 'MB', 'GB', 'TB', 'PB')
    $value = [double]$Bytes
    $idx = 0

    while ($value -ge 1024 -and $idx -lt ($units.Count - 1)) {
        $value /= 1024
        $idx++
    }

    if ($idx -eq 0) {
        return "{0} {1}" -f [math]::Round($value, 0), $units[$idx]
    }

    return "{0:N2} {1}" -f $value, $units[$idx]
}

function Trim-Label {
    param(
        [string]$Text,
        [int]$Max = 34
    )

    if ([string]::IsNullOrWhiteSpace($Text)) { return '(vide)' }
    if ($Text.Length -le $Max) { return $Text }
    return $Text.Substring(0, $Max - 1) + '…'
}

function Get-NodeTree {
    param(
        [string]$TargetPath,
        [int]$CurrentLevel,
        [int]$MaxVisualLevel
    )

    try {
        $item = Get-Item -LiteralPath $TargetPath -Force -ErrorAction Stop
    }
    catch {
        Add-ScanError -TargetPath $TargetPath -Operation 'GetItem' -Exception $_.Exception
        return New-ScanNode -TargetPath $TargetPath -Name (Split-Path -Leaf $TargetPath) -IsDirectory $true -Level $CurrentLevel -Inaccessible $true
    }

    if (-not $item.PSIsContainer) {
        try {
            $fileSize = [int64]$item.Length
        }
        catch {
            Add-ScanError -TargetPath $TargetPath -Operation 'ReadFileLength' -Exception $_.Exception
            $fileSize = 0
        }

        return New-ScanNode -TargetPath $TargetPath -Name $item.Name -IsDirectory $false -Level $CurrentLevel -Size $fileSize
    }

    $name = if ([string]::IsNullOrWhiteSpace($item.Name)) { $item.FullName } else { $item.Name }
    $node = New-ScanNode -TargetPath $item.FullName -Name $name -IsDirectory $true -Level $CurrentLevel

    try {
        $children = Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop
    }
    catch {
        Add-ScanError -TargetPath $item.FullName -Operation 'ListDirectory' -Exception $_.Exception
        $node.Inaccessible = $true
        return $node
    }

    foreach ($child in $children) {
        if ($child.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            continue
        }

        if ($child.PSIsContainer) {
            $childNode = Get-NodeTree -TargetPath $child.FullName -CurrentLevel ($CurrentLevel + 1) -MaxVisualLevel $MaxVisualLevel
            $node.Size += [int64]$childNode.Size

            if ($CurrentLevel -lt $MaxVisualLevel) {
                $null = $node.Children.Add($childNode)
            }
        }
        else {
            $fileSize = 0
            try {
                $fileSize = [int64]$child.Length
            }
            catch {
                Add-ScanError -TargetPath $child.FullName -Operation 'ReadFileLength' -Exception $_.Exception
            }

            $node.Size += $fileSize

            if ($IncludeFiles -and $CurrentLevel -lt $MaxVisualLevel) {
                $fileNode = New-ScanNode -TargetPath $child.FullName -Name $child.Name -IsDirectory $false -Level ($CurrentLevel + 1) -Size $fileSize
                $null = $node.Children.Add($fileNode)
            }
        }
    }

    return $node
}

function Get-NodesAtLevel {
    param(
        [object]$Root,
        [int]$TargetLevel
    )

    $results = New-Object System.Collections.Generic.List[object]

    function VisitNode {
        param(
            [object]$Node,
            [int]$Target,
            [System.Collections.Generic.List[object]]$Store
        )

        if ($Node.Level -eq $Target) {
            $null = $Store.Add($Node)
        }

        foreach ($child in $Node.Children) {
            VisitNode -Node $child -Target $Target -Store $Store
        }
    }

    VisitNode -Node $Root -Target $TargetLevel -Store $results
    return $results
}

function Show-Gauges {
    param(
        [object[]]$Items,
        [int64]$LevelTotal
    )

    $barWidth = 48
    foreach ($item in $Items) {
        $percent = if ($LevelTotal -gt 0) { ($item.Size / $LevelTotal) * 100 } else { 0 }
        $length = [math]::Round(($percent / 100) * $barWidth)
        $length = [Math]::Max([Math]::Min($length, $barWidth), 0)

        $barMain = ('█' * $length)
        $barPad = ('░' * ($barWidth - $length))
        $bar = $barMain + $barPad

        Write-Host ("{0,-35} {1,10} {2,6:N2}% |{3}|" -f (Trim-Label $item.Name 35), (Format-Size $item.Size), $percent, $bar)
    }
}

function Show-3DColumns {
    param(
        [object[]]$Items,
        [int64]$LevelTotal
    )

    if (-not $Items -or $Items.Count -eq 0) {
        Write-Host 'Aucun élément à afficher en vue 3D.'
        return
    }

    $height = 10
    $maxSize = ($Items | Measure-Object -Property Size -Maximum).Maximum
    if ($maxSize -lt 1) { $maxSize = 1 }

    $levels = @()
    foreach ($item in $Items) {
        $levels += [math]::Round(($item.Size / $maxSize) * $height)
    }

    for ($row = $height; $row -ge 1; $row--) {
        $line = ''
        for ($i = 0; $i -lt $Items.Count; $i++) {
            if ($levels[$i] -ge $row) {
                $line += ' █▓ '
            }
            else {
                $line += '    '
            }
        }

        Write-Host $line
    }

    $indexLine = ''
    for ($i = 1; $i -le $Items.Count; $i++) {
        $indexLine += (' {0:D2} ' -f $i)
    }
    Write-Host $indexLine

    for ($i = 0; $i -lt $Items.Count; $i++) {
        $percent = if ($LevelTotal -gt 0) { ($Items[$i].Size / $LevelTotal) * 100 } else { 0 }
        Write-Host ("{0,2}) {1,-35} {2,10} {3,6:N2}%" -f ($i + 1), (Trim-Label $Items[$i].Name 35), (Format-Size $Items[$i].Size), $percent)
    }
}

if (-not (Test-Path -LiteralPath $Path)) {
    Write-Error "Le chemin '$Path' est introuvable."
    exit 1
}

try {
    $resolvedPath = (Resolve-Path -LiteralPath $Path -ErrorAction Stop).Path
}
catch {
    Write-Error "Impossible de résoudre le chemin '$Path'. Détail: $($_.Exception.Message)"
    exit 1
}

$scanStartedAt = Get-Date
Write-Host "Analyse de '$resolvedPath' en cours..." -ForegroundColor Cyan
Write-Host "Mode visuel: $VisualMode | Niveau max: $Depth | Top: $Top" -ForegroundColor DarkCyan

$root = Get-NodeTree -TargetPath $resolvedPath -CurrentLevel 0 -MaxVisualLevel $Depth

$elapsed = (Get-Date) - $scanStartedAt
Write-Host ''
Write-Host '=== Résumé global ===' -ForegroundColor Green
Write-Host ("Racine analysée : {0}" -f $root.Path)
Write-Host ("Taille totale   : {0}" -f (Format-Size $root.Size))
Write-Host ("Durée           : {0:N1}s" -f $elapsed.TotalSeconds)
Write-Host ("Erreurs captées : {0}" -f $script:ScanErrors.Count)

for ($level = 1; $level -le $Depth; $level++) {
    $nodes = @(Get-NodesAtLevel -Root $root -TargetLevel $level | Where-Object { $_.IsDirectory -or $IncludeFiles })

    if ($nodes.Count -eq 0) {
        Write-Host ''
        Write-Host ("=== Niveau {0} : aucun élément disponible ===" -f $level) -ForegroundColor Yellow
        continue
    }

    $ordered = @($nodes | Sort-Object -Property Size -Descending)
    $topItems = @($ordered | Select-Object -First $Top)
    $levelTotal = [int64](($ordered | Measure-Object -Property Size -Sum).Sum)

    Write-Host ''
    Write-Host ("=== Niveau {0} | Top {1} éléments ===" -f $level, $topItems.Count) -ForegroundColor Magenta

    if ($VisualMode -in @('Gauge', 'Both')) {
        Write-Host '-- Vue jauges --' -ForegroundColor DarkMagenta
        Show-Gauges -Items $topItems -LevelTotal $levelTotal
    }

    if ($VisualMode -in @('3D', 'Both')) {
        Write-Host ''
        Write-Host '-- Vue colonnes 3D --' -ForegroundColor DarkMagenta
        Show-3DColumns -Items $topItems -LevelTotal $levelTotal
    }

    Write-Host ''
    Write-Host '-- Détails --' -ForegroundColor DarkMagenta
    foreach ($item in $topItems) {
        $pct = if ($levelTotal -gt 0) { ($item.Size / $levelTotal) * 100 } else { 0 }
        Write-Host ("- {0} | {1:N2}% | {2} | {3}" -f (Format-Size $item.Size), $pct, $item.Path, ($(if ($item.Inaccessible) { 'Accès partiel' } else { 'OK' })))
    }
}

if ($script:ScanErrors.Count -gt 0) {
    Write-Host ''
    Write-Host '=== Erreurs rencontrées (extrait) ===' -ForegroundColor Yellow
    $preview = $script:ScanErrors | Select-Object -First 20
    foreach ($err in $preview) {
        Write-Host ("[{0}] {1} -> {2} ({3})" -f $err.Operation, $err.Path, $err.Message, $err.Type)
    }

    if ($script:ScanErrors.Count -gt $preview.Count) {
        Write-Host ("... {0} erreurs supplémentaires non affichées." -f ($script:ScanErrors.Count - $preview.Count))
    }
}

Write-Host ''
Write-Host 'Analyse terminée.' -ForegroundColor Green
