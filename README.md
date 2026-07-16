# WinSpace

WinSpace est une application Windows locale qui permet d’identifier et d’explorer ce qui occupe l’espace disque.

## Application graphique

- Sélection d’un disque ou d’un dossier à analyser
- Analyse parallèle avec progression, erreurs d’accès et annulation
- Carte de bulles proportionnelle à l’espace occupé
- Navigation dans les dossiers, recherche et tri
- Ouverture dans l’Explorateur Windows et copie du chemin
- Thème clair/sombre adapté au système

## Lancer en développement

Prérequis : Node.js 22+, Rust stable et les outils de compilation Visual Studio pour C++.

```powershell
npm install
npm run tauri -- dev
```

## Construire l’installeur Windows

```powershell
npm run tauri -- build
```

L’installeur NSIS est généré dans `src-tauri/target/release/bundle/nsis/`.

## Ancienne version PowerShell

`WinSpace.ps1` est conservé comme référence du prototype initial. La version graphique ne dépend pas de ce script.
