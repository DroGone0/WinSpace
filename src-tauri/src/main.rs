use jwalk::{Parallelism, WalkDir};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

#[derive(Clone, Serialize)]
struct Drive { path: String, label: String }

#[derive(Clone, Serialize)]
struct NodeSummary {
    path: String,
    name: String,
    size: u64,
    is_dir: bool,
    child_count: usize,
    inaccessible: bool,
}

#[derive(Clone, Serialize)]
struct ScanProgress {
    scan_id: String,
    entries: u64,
    files: u64,
    directories: u64,
    errors: u64,
    current_path: String,
    elapsed_ms: u128,
}

#[derive(Clone, Serialize)]
struct ScanComplete { scan_id: String, root: NodeSummary, entries: u64, errors: u64, elapsed_ms: u128 }
#[derive(Clone, Serialize)]
struct ScanFailure { scan_id: String, message: String }

#[derive(Clone)]
struct Entry { path: PathBuf, name: String, size: u64, is_dir: bool, inaccessible: bool }

struct ScanResult { root: PathBuf, entries: HashMap<PathBuf, Entry> }

#[derive(Default)]
struct ScannerState {
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
    results: Mutex<HashMap<String, ScanResult>>,
}

fn display_name(path: &Path) -> String {
    path.file_name().and_then(|name| name.to_str()).map(String::from).unwrap_or_else(|| path.display().to_string())
}

fn summary(entry: &Entry, child_count: usize) -> NodeSummary {
    NodeSummary { path: entry.path.display().to_string(), name: entry.name.clone(), size: entry.size, is_dir: entry.is_dir, child_count, inaccessible: entry.inaccessible }
}

fn child_count(entries: &HashMap<PathBuf, Entry>, path: &Path) -> usize {
    entries.values().filter(|item| item.path.parent() == Some(path)).count()
}

#[tauri::command]
fn list_drives() -> Vec<Drive> {
    (b'A'..=b'Z').filter_map(|letter| {
        let path = format!("{}:\\", letter as char);
        Path::new(&path).exists().then(|| Drive { label: format!("Disque local ({}:)", letter as char), path })
    }).collect()
}

#[tauri::command]
fn start_scan(app: AppHandle, state: State<'_, ScannerState>, path: String) -> Result<String, String> {
    let root = fs::canonicalize(&path).map_err(|error| format!("Impossible d’ouvrir cet emplacement : {error}"))?;
    let scan_id = Uuid::new_v4().to_string();
    let cancelled = Arc::new(AtomicBool::new(false));
    state.cancellations.lock().map_err(|_| "État du scan indisponible".to_string())?.insert(scan_id.clone(), cancelled.clone());
    let app_handle = app.clone();
    let scan_id_for_thread = scan_id.clone();
    thread::spawn(move || run_scan(app_handle, scan_id_for_thread, root, cancelled));
    Ok(scan_id)
}

fn run_scan(app: AppHandle, scan_id: String, root: PathBuf, cancelled: Arc<AtomicBool>) {
    let started = Instant::now();
    let mut entries: HashMap<PathBuf, Entry> = HashMap::new();
    entries.insert(root.clone(), Entry { path: root.clone(), name: display_name(&root), size: 0, is_dir: true, inaccessible: false });
    let mut count = 0_u64; let mut files = 0_u64; let mut directories = 1_u64; let mut errors = 0_u64; let mut last_event = Instant::now();
    let workers = thread::available_parallelism().map(|number| number.get().clamp(2, 8)).unwrap_or(4);
    for item in WalkDir::new(&root).skip_hidden(false).parallelism(Parallelism::RayonNewPool(workers)) {
        if cancelled.load(Ordering::Relaxed) { return; }
        match item {
            Ok(dir_entry) => {
                let path = dir_entry.path();
                if path == root { continue; }
                let metadata = match dir_entry.metadata() { Ok(value) => value, Err(_) => { errors += 1; continue; } };
                let is_dir = metadata.is_dir();
                let size = if is_dir { 0 } else { metadata.len() };
                if is_dir { directories += 1; } else { files += 1; }
                entries.insert(path.to_path_buf(), Entry { path: path.to_path_buf(), name: display_name(&path), size, is_dir, inaccessible: false });
                count += 1;
                if last_event.elapsed() >= Duration::from_millis(160) {
                    let _ = app.emit("scan-progress", ScanProgress { scan_id: scan_id.clone(), entries: count, files, directories, errors, current_path: path.display().to_string(), elapsed_ms: started.elapsed().as_millis() });
                    last_event = Instant::now();
                }
            }
            Err(_) => errors += 1,
        }
    }
    for item in entries.values().filter(|entry| !entry.is_dir).cloned().collect::<Vec<_>>() {
        let mut parent = item.path.parent();
        while let Some(folder) = parent {
            if let Some(directory) = entries.get_mut(folder) { directory.size = directory.size.saturating_add(item.size); }
            if folder == root { break; }
            parent = folder.parent();
        }
    }
    let root_entry = match entries.get(&root) { Some(entry) => entry.clone(), None => { let _ = app.emit("scan-failed", ScanFailure { scan_id, message: "Aucun résultat n’a été produit.".into() }); return; } };
    let root_summary = summary(&root_entry, child_count(&entries, &root));
    if let Some(state) = app.try_state::<ScannerState>() {
        if let Ok(mut results) = state.results.lock() { results.insert(scan_id.clone(), ScanResult { root, entries }); }
        if let Ok(mut cancellations) = state.cancellations.lock() { cancellations.remove(&scan_id); }
    }
    let _ = app.emit("scan-complete", ScanComplete { scan_id, root: root_summary, entries: count, errors, elapsed_ms: started.elapsed().as_millis() });
}

#[tauri::command]
fn cancel_scan(state: State<'_, ScannerState>, scan_id: String) {
    if let Ok(cancellations) = state.cancellations.lock() { if let Some(token) = cancellations.get(&scan_id) { token.store(true, Ordering::Relaxed); } }
}

#[tauri::command]
fn get_children(state: State<'_, ScannerState>, scan_id: String, path: String) -> Result<Vec<NodeSummary>, String> {
    let path = PathBuf::from(path);
    let results = state.results.lock().map_err(|_| "Résultats indisponibles".to_string())?;
    let result = results.get(&scan_id).ok_or("Analyse introuvable")?;
    if !path.starts_with(&result.root) { return Err("Chemin hors de la cible analysée".into()); }
    let mut children: Vec<_> = result.entries.values().filter(|entry| entry.path.parent() == Some(path.as_path())).map(|entry| summary(entry, child_count(&result.entries, &entry.path))).collect();
    children.sort_by(|left, right| right.size.cmp(&left.size));
    Ok(children)
}

#[tauri::command]
fn open_in_explorer(path: String) -> Result<(), String> {
    if !Path::new(&path).exists() { return Err("Cet élément n’existe plus.".into()); }
    std::process::Command::new("explorer.exe").arg(path).spawn().map_err(|error| error.to_string())?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(ScannerState::default())
        .invoke_handler(tauri::generate_handler![list_drives, start_scan, cancel_scan, get_children, open_in_explorer])
        .run(tauri::generate_context!())
        .expect("Erreur au démarrage de WinSpace");
}
