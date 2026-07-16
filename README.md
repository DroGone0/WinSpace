# WinSpace
Outil PowerShell pour visualiser rapidement ce qui prend le plus d'espace disque sur Windows, avec une vue **détaillée** et **très visuelle**.

## Fonctionnalités (v1)
- Analyse d'un chemin disque (par défaut: disque système)
- Visualisation multi-niveaux (`-Depth`) des dossiers les plus volumineux
- Vue **jauges** + vue **colonnes 3D** (ASCII/Unicode) pour un rendu visuel immédiat
- Détails par élément (taille, pourcentage, chemin, état d'accès)
- Gestion des erreurs: accès refusé, lecture impossible, chemins invalides, etc.
- Compatible PowerShell Windows (Windows PowerShell 5.1+ et PowerShell 7+)

## Utilisation
Depuis le dossier du projet:

```powershell
powershell -ExecutionPolicy Bypass -File .\WinSpace.ps1
```

Exemples:

```powershell
# Analyse C:\ sur 3 niveaux, top 8, vues jauges + 3D
powershell -ExecutionPolicy Bypass -File .\WinSpace.ps1 -Path C:\ -Depth 3 -Top 8 -VisualMode Both

# Vue uniquement jauges
powershell -ExecutionPolicy Bypass -File .\WinSpace.ps1 -Path D:\ -VisualMode Gauge

# Inclure les fichiers (pas seulement dossiers) dans les vues
powershell -ExecutionPolicy Bypass -File .\WinSpace.ps1 -Path C:\Users -Depth 2 -Top 10 -IncludeFiles
```

## Paramètres
- `-Path` : chemin à analyser
- `-Depth` : niveau de détail (1 à 8)
- `-Top` : nombre d'éléments affichés par niveau (1 à 20)
- `-VisualMode` : `Gauge`, `3D`, ou `Both`
- `-IncludeFiles` : inclut aussi les fichiers dans l'affichage par niveau
