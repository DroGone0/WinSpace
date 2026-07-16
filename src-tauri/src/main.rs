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

const MAX_TRACKED_DIRECTORIES: usize = 200_000;
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(450);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(160);

#[derive(Clone, Serialize)]
struct Drive { path: String, label: String }

#[derive(Clone, Serialize)]
struct NodeSummary {
    path: String,
    name: String,
    size: u64,
    is_dir: bool,
    is_virtual: bool,
    child_count: usize,
    inaccessible: bool,
}

#[derive(Clone, Serialize)]
struct CategorySummary { id: String, label: String, size: u64, file_count: u64 }

#[derive(Default)]
struct CategoryTotals(HashMap<&'static str, (u64, u64)>);

impl CategoryTotals {
    fn add(&mut self, path: &Path, size: u64) {
        let id = classify_file(path);
        let total = self.0.entry(id).or_default(); total.0 = total.0.saturating_add(size); total.1 += 1;
    }
    fn summaries(&self) -> Vec<CategorySummary> {
        let mut values: Vec<_> = self.0.iter().map(|(id, (size, file_count))| CategorySummary { id: (*id).into(), label: category_label(id).into(), size: *size, file_count: *file_count }).collect();
        values.sort_by(|left, right| right.size.cmp(&left.size)); values
    }
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
    tracked_directories: usize,
    capped: bool,
}

#[derive(Clone, Serialize)]
struct ScanSnapshot { scan_id: String, root: NodeSummary, children: Vec<NodeSummary>, categories: Vec<CategorySummary>, entries: u64, errors: u64, elapsed_ms: u128, capped: bool }
#[derive(Clone, Serialize)]
struct ScanComplete { scan_id: String, root: NodeSummary, categories: Vec<CategorySummary>, entries: u64, errors: u64, elapsed_ms: u128, capped: bool }
#[derive(Clone, Serialize)]
struct ScanFailure { scan_id: String, message: String }

#[derive(Clone)]
struct Entry { path: PathBuf, name: String, size: u64, direct_files_size: u64, direct_file_count: u64, inaccessible: bool }
struct ScanResult { root: PathBuf, entries: HashMap<PathBuf, Entry> }

#[derive(Default)]
struct ScannerState { cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>, results: Mutex<HashMap<String, ScanResult>> }

fn visible_path(path: &Path) -> String {
    let value = path.display().to_string();
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
}

fn display_name(path: &Path) -> String {
    path.file_name().and_then(|name| name.to_str()).map(String::from).unwrap_or_else(|| visible_path(path))
}

fn classify_file(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "raw" | "cr2" | "svg" | "bmp" | "tiff" => "photos",
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "m4v" => "videos",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => "audio",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "odt" | "csv" => "documents",
        "zip" | "rar" | "7z" | "tar" | "gz" | "iso" | "cab" => "archives",
        "exe" | "msi" | "dll" | "appx" | "sys" => "applications",
        "js" | "ts" | "tsx" | "jsx" | "rs" | "py" | "java" | "cs" | "cpp" | "html" | "css" | "json" => "code",
        _ => "other",
    }
}

fn category_label(id: &str) -> &'static str {
    match id { "photos" => "Photos", "videos" => "Vidéos", "audio" => "Audio", "documents" => "Documents", "archives" => "Archives", "applications" => "Applications", "code" => "Code", _ => "Autres fichiers" }
}

fn child_count(entries: &HashMap<PathBuf, Entry>, path: &Path) -> usize {
    entries.values().filter(|item| item.path.parent() == Some(path)).count()
}

fn summary(entry: &Entry, children: usize) -> NodeSummary {
    NodeSummary { path: entry.path.display().to_string(), name: entry.name.clone(), size: entry.size, is_dir: true, is_virtual: false, child_count: children, inaccessible: entry.inaccessible }
}

fn direct_files_summary(entry: &Entry) -> Option<NodeSummary> {
    (entry.direct_file_count > 0).then(|| NodeSummary {
        path: format!("{}::__files", entry.path.display()), name: "Fichiers directs".into(), size: entry.direct_files_size,
        is_dir: false, is_virtual: true, child_count: entry.direct_file_count as usize, inaccessible: false,
    })
}

fn ensure_directory(entries: &mut HashMap<PathBuf, Entry>, path: &Path, capped: &mut bool) {
    if entries.contains_key(path) { return; }
    if entries.len() >= MAX_TRACKED_DIRECTORIES { *capped = true; return; }
    entries.insert(path.to_path_buf(), Entry { path: path.to_path_buf(), name: display_name(path), size: 0, direct_files_size: 0, direct_file_count: 0, inaccessible: false });
}

fn add_file_size(entries: &mut HashMap<PathBuf, Entry>, root: &Path, file: &Path, size: u64) {
    let mut parent = file.parent();
    let mut immediate_parent = true;
    while let Some(folder) = parent {
        if let Some(directory) = entries.get_mut(folder) {
            directory.size = directory.size.saturating_add(size);
            if immediate_parent {
                directory.direct_files_size = directory.direct_files_size.saturating_add(size);
                directory.direct_file_count += 1;
            }
        }
        immediate_parent = false;
        if folder == root { break; }
        parent = folder.parent();
    }
}

fn root_children(entries: &HashMap<PathBuf, Entry>, root: &Path) -> Vec<NodeSummary> {
    let mut children: Vec<_> = entries.values().filter(|item| item.path.parent() == Some(root)).map(|item| summary(item, child_count(entries, &item.path))).collect();
    if let Some(root_entry) = entries.get(root) { if let Some(files) = direct_files_summary(root_entry) { children.push(files); } }
    children.sort_by(|left, right| right.size.cmp(&left.size));
    children.truncate(64);
    children
}

fn snapshot(scan_id: &str, entries: &HashMap<PathBuf, Entry>, categories: &CategoryTotals, root: &Path, count: u64, errors: u64, started: Instant, capped: bool) -> Option<ScanSnapshot> {
    let root_entry = entries.get(root)?;
    Some(ScanSnapshot { scan_id: scan_id.into(), root: summary(root_entry, child_count(entries, root)), children: root_children(entries, root), categories: categories.summaries(), entries: count, errors, elapsed_ms: started.elapsed().as_millis(), capped })
}

#[tauri::command]
fn list_drives() -> Vec<Drive> {
    (b'A'..=b'Z').filter_map(|letter| { let path = format!("{}:\\", letter as char); Path::new(&path).exists().then(|| Drive { label: format!("Disque local ({}:)", letter as char), path }) }).collect()
}

#[tauri::command]
fn start_scan(app: AppHandle, state: State<'_, ScannerState>, path: String) -> Result<String, String> {
    let root = fs::canonicalize(&path).map_err(|error| format!("Impossible d’ouvrir cet emplacement : {error}"))?;
    let scan_id = Uuid::new_v4().to_string();
    let cancelled = Arc::new(AtomicBool::new(false));
    state.cancellations.lock().map_err(|_| "État du scan indisponible".to_string())?.insert(scan_id.clone(), cancelled.clone());
    let thread_id = scan_id.clone();
    thread::spawn(move || run_scan(app, thread_id, root, cancelled));
    Ok(scan_id)
}

fn run_scan(app: AppHandle, scan_id: String, root: PathBuf, cancelled: Arc<AtomicBool>) {
    let started = Instant::now();
    let mut entries = HashMap::new(); let mut capped = false; let mut categories = CategoryTotals::default();
    ensure_directory(&mut entries, &root, &mut capped);
    if let Ok(children) = fs::read_dir(&root) { for child in children.flatten() { if child.file_type().map(|kind| kind.is_dir()).unwrap_or(false) { ensure_directory(&mut entries, &child.path(), &mut capped); } } }
    if let Some(initial) = snapshot(&scan_id, &entries, &categories, &root, 0, 0, started, capped) { let _ = app.emit("scan-snapshot", initial); }
    let workers = thread::available_parallelism().map(|number| number.get().clamp(2, 6)).unwrap_or(4);
    let mut count = 0_u64; let mut files = 0_u64; let mut directories = entries.len() as u64; let mut errors = 0_u64;
    let mut last_progress = Instant::now(); let mut last_snapshot = Instant::now();
    for item in WalkDir::new(&root).skip_hidden(false).parallelism(Parallelism::RayonNewPool(workers)) {
        if cancelled.load(Ordering::Relaxed) {
            if let Some(partial) = snapshot(&scan_id, &entries, &categories, &root, count, errors, started, capped) { let _ = app.emit("scan-cancelled", partial); }
            if let Some(state) = app.try_state::<ScannerState>() { if let Ok(mut cancellations) = state.cancellations.lock() { cancellations.remove(&scan_id); } }
            return;
        }
        match item {
            Ok(dir_entry) => {
                let path = dir_entry.path();
                if path == root { continue; }
                match dir_entry.metadata() {
                    Ok(metadata) if metadata.is_dir() => { let known = entries.contains_key(&path); ensure_directory(&mut entries, &path, &mut capped); if !known { directories += 1; } }
                    Ok(metadata) => { files += 1; add_file_size(&mut entries, &root, &path, metadata.len()); categories.add(&path, metadata.len()); }
                    Err(_) => errors += 1,
                }
                count += 1;
                if last_progress.elapsed() >= PROGRESS_INTERVAL {
                    let _ = app.emit("scan-progress", ScanProgress { scan_id: scan_id.clone(), entries: count, files, directories, errors, current_path: visible_path(&path), elapsed_ms: started.elapsed().as_millis(), tracked_directories: entries.len(), capped });
                    last_progress = Instant::now();
                }
                if last_snapshot.elapsed() >= SNAPSHOT_INTERVAL {
                    if let Some(partial) = snapshot(&scan_id, &entries, &categories, &root, count, errors, started, capped) { let _ = app.emit("scan-snapshot", partial); }
                    last_snapshot = Instant::now();
                }
            }
            Err(_) => errors += 1,
        }
    }
    let root_entry = match entries.get(&root) { Some(entry) => entry.clone(), None => { let _ = app.emit("scan-failed", ScanFailure { scan_id, message: "Aucun résultat n’a été produit.".into() }); return; } };
    let root_summary = summary(&root_entry, child_count(&entries, &root));
    if let Some(state) = app.try_state::<ScannerState>() {
        if let Ok(mut results) = state.results.lock() { results.insert(scan_id.clone(), ScanResult { root, entries }); }
        if let Ok(mut cancellations) = state.cancellations.lock() { cancellations.remove(&scan_id); }
    }
    let _ = app.emit("scan-complete", ScanComplete { scan_id, root: root_summary, categories: categories.summaries(), entries: count, errors, elapsed_ms: started.elapsed().as_millis(), capped });
}

#[tauri::command]
fn cancel_scan(state: State<'_, ScannerState>, scan_id: String) { if let Ok(cancellations) = state.cancellations.lock() { if let Some(token) = cancellations.get(&scan_id) { token.store(true, Ordering::Relaxed); } } }

#[tauri::command]
fn get_children(state: State<'_, ScannerState>, scan_id: String, path: String) -> Result<Vec<NodeSummary>, String> {
    let path = PathBuf::from(path); let results = state.results.lock().map_err(|_| "Résultats indisponibles".to_string())?; let result = results.get(&scan_id).ok_or("Analyse introuvable")?;
    if !path.starts_with(&result.root) { return Err("Chemin hors de la cible analysée".into()); }
    let mut children: Vec<_> = result.entries.values().filter(|entry| entry.path.parent() == Some(path.as_path())).map(|entry| summary(entry, child_count(&result.entries, &entry.path))).collect();
    if let Some(entry) = result.entries.get(&path) { if let Some(files) = direct_files_summary(entry) { children.push(files); } }
    children.sort_by(|left, right| right.size.cmp(&left.size)); Ok(children)
}

#[tauri::command]
fn open_in_explorer(path: String) -> Result<(), String> { if !Path::new(&path).exists() { return Err("Cet élément n’existe plus.".into()); } std::process::Command::new("explorer.exe").arg(path).spawn().map_err(|error| error.to_string())?; Ok(()) }

fn main() {
    tauri::Builder::default().plugin(tauri_plugin_dialog::init()).manage(ScannerState::default())
        .invoke_handler(tauri::generate_handler![list_drives, start_scan, cancel_scan, get_children, open_in_explorer])
        .run(tauri::generate_context!()).expect("Erreur au démarrage de WinSpace");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_size_is_aggregated_to_each_tracked_parent() {
        let root = PathBuf::from("C:/scan"); let folder = root.join("Users"); let file = folder.join("archive.zip"); let mut entries = HashMap::new(); let mut capped = false;
        ensure_directory(&mut entries, &root, &mut capped); ensure_directory(&mut entries, &folder, &mut capped); add_file_size(&mut entries, &root, &file, 42);
        assert_eq!(entries.get(&root).unwrap().size, 42); assert_eq!(entries.get(&folder).unwrap().size, 42); assert_eq!(entries.get(&folder).unwrap().direct_files_size, 42);
    }
}
