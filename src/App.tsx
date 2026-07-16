import { useEffect, useMemo, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import {
  ArrowLeft, ChevronRight, Copy, FolderOpen, HardDrive, LoaderCircle,
  Search, Settings2, Sparkles, X, Image, Video, Music, FileText, Archive, AppWindow, Code2, File
} from 'lucide-react'

type Drive = { path: string; label: string }
type Node = { path: string; name: string; size: number; is_dir: boolean; is_virtual: boolean; child_count: number; inaccessible: boolean }
type Progress = { scan_id: string; entries: number; files: number; directories: number; errors: number; current_path: string; elapsed_ms: number; tracked_directories: number; capped: boolean }
type Category = { id: string; label: string; size: number; file_count: number }
type Snapshot = { scan_id: string; root: Node; children: Node[]; categories: Category[]; entries: number; errors: number; elapsed_ms: number; capped: boolean }
type Complete = { scan_id: string; root: Node; categories: Category[]; entries: number; errors: number; elapsed_ms: number; capped: boolean }

const demo: Node[] = [
  ['Users', 168_000_000_000, 142_220], ['Windows', 39_800_000_000, 91_420], ['Program Files', 27_400_000_000, 28_120],
  ['ProgramData', 14_600_000_000, 12_300], ['Games', 82_000_000_000, 4_221], ['Downloads', 9_600_000_000, 4_110],
  ['Temp', 3_200_000_000, 2_001], ['Recovery', 1_000_000_000, 18]
].map(([name, size, child_count]) => ({ path: `C:\\${name}`, name: String(name), size: Number(size), child_count: Number(child_count), is_dir: true, is_virtual: false, inaccessible: false }))

const isTauri = () => '__TAURI_INTERNALS__' in window
const formatSize = (bytes: number) => {
  const units = ['o', 'Ko', 'Mo', 'Go', 'To']; let value = Math.max(0, bytes); let index = 0
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index++ }
  return `${value.toLocaleString('fr-FR', { maximumFractionDigits: index ? 1 : 0 })} ${units[index]}`
}
const elapsed = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}`
const colorFor = (name: string, index: number) => ['#6c7cff', '#a46cff', '#30d5c8', '#ff9d6c', '#ed6aa6', '#f0cc67', '#48a8ff', '#8dc16c'][(name.length + index) % 8]
const priorityFor = (name: string) => /downloads|temp|cache|recycle|appdata|programdata/i.test(name) ? 'À vérifier' : ''
const categoryIcon = (id: string) => ({ photos: Image, videos: Video, audio: Music, documents: FileText, archives: Archive, applications: AppWindow, code: Code2 }[id] || File)

export default function App() {
  const [drives, setDrives] = useState<Drive[]>([{ path: 'C:\\', label: 'Disque local (C:)' }])
  const [target, setTarget] = useState('C:\\')
  const [phase, setPhase] = useState<'home' | 'scanning' | 'results'>('home')
  const [scanId, setScanId] = useState<string | null>(null)
  const [isScanning, setIsScanning] = useState(false)
  const [capped, setCapped] = useState(false)
  const [categories, setCategories] = useState<Category[]>([])
  const [history, setHistory] = useState<Node[]>([])
  const [progress, setProgress] = useState<Progress | null>(null)
  const [root, setRoot] = useState<Node | null>(null)
  const [current, setCurrent] = useState<Node | null>(null)
  const [items, setItems] = useState<Node[]>([])
  const [selected, setSelected] = useState<Node | null>(null)
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<'size' | 'name'>('size')
  const [error, setError] = useState('')

  useEffect(() => {
    if (!isTauri()) { setItems(demo); return }
    invoke<Drive[]>('list_drives').then(setDrives).catch(() => undefined)
    const unlisten = Promise.all([
      listen<Progress>('scan-progress', ({ payload }) => setProgress(payload)),
      listen<Snapshot>('scan-snapshot', ({ payload }) => {
        setScanId(payload.scan_id); setRoot(payload.root); setCurrent(current => current ?? payload.root); setItems(payload.children); setCategories(payload.categories); setPhase('results'); setIsScanning(true); setCapped(payload.capped)
      }),
      listen<Complete>('scan-complete', ({ payload }) => {
        setRoot(payload.root); setCurrent(payload.root); setSelected(null); setHistory([]); setCategories(payload.categories); setPhase('results'); setProgress(null); setIsScanning(false); setCapped(payload.capped)
        void loadChildren(payload.scan_id, payload.root.path)
      }),
      listen<Snapshot>('scan-cancelled', ({ payload }) => { setRoot(payload.root); setCurrent(payload.root); setItems(payload.children); setPhase('results'); setIsScanning(false); setProgress(null); setCapped(payload.capped) }),
      listen<{ scan_id: string; message: string }>('scan-failed', ({ payload }) => { setError(payload.message); setPhase('home') })
    ])
    return () => { void unlisten.then(values => values.forEach(fn => fn())) }
  }, [])

  const loadChildren = async (id: string, path: string) => {
    if (!isTauri()) { setItems(demo); return }
    try { setItems(await invoke<Node[]>('get_children', { scanId: id, path })); } catch { setItems([]) }
  }
  const start = async () => {
    setError(''); setPhase('scanning'); setProgress(null); setIsScanning(true); setCapped(false); setCurrent(null); setItems([]); setHistory([]); setCategories([])
    if (!isTauri()) { window.setTimeout(() => { const r = { path: target, name: target, size: demo.reduce((sum, node) => sum + node.size, 0), is_dir: true, is_virtual: false, child_count: demo.length, inaccessible: false }; setScanId('demo'); setRoot(r); setCurrent(r); setItems(demo); setCategories([{ id: 'applications', label: 'Applications', size: 135_000_000_000, file_count: 2022 }, { id: 'photos', label: 'Photos', size: 71_000_000_000, file_count: 4211 }, { id: 'videos', label: 'Vidéos', size: 47_000_000_000, file_count: 421 }, { id: 'documents', label: 'Documents', size: 12_000_000_000, file_count: 8200 }]); setPhase('results'); setIsScanning(false) }, 950); return }
    try { const id = await invoke<string>('start_scan', { path: target }); setScanId(id) } catch (reason) { setError(String(reason)); setPhase('home') }
  }
  const cancel = async () => { if (scanId && isTauri()) await invoke('cancel_scan', { scanId }); else setPhase('home') }
  const chooseFolder = async () => {
    const selectedPath = await open({ directory: true, multiple: false, title: 'Choisir un dossier à analyser' })
    if (typeof selectedPath === 'string') setTarget(selectedPath)
  }
  const openNode = async (node: Node) => {
    if (!node.is_dir || !scanId || isScanning) return
    if (current) setHistory(previous => [...previous, current].slice(-80))
    setCurrent(node); setSelected(null); await loadChildren(scanId, node.path)
  }
  const back = async () => {
    const previous = history.at(-1)
    if (!previous || !scanId) return
    setHistory(items => items.slice(0, -1)); setCurrent(previous); setSelected(null); await loadChildren(scanId, previous.path)
  }
  const filtered = useMemo(() => items.filter(item => item.name.toLocaleLowerCase().includes(query.toLocaleLowerCase())).sort((a, b) => sort === 'size' ? b.size - a.size : a.name.localeCompare(b.name)), [items, query, sort])
  const total = current?.size || filtered.reduce((sum, node) => sum + node.size, 0)
  const copyPath = async () => { if (selected) await navigator.clipboard.writeText(selected.path) }
  const reveal = async () => { if (selected && !selected.is_virtual && isTauri()) await invoke('open_in_explorer', { path: selected.path }) }

  if (phase === 'home') return <main className="welcome-shell">
    <section className="welcome-card">
      <div className="brand-mark"><Sparkles size={22} /></div><span className="eyebrow">WINSPACE / LOCAL ANALYZER</span>
      <h1>Voyez ce qui prend<br />vraiment de la place.</h1>
      <p>Une lecture claire de votre stockage, sans envoyer un seul fichier hors de votre ordinateur.</p>
      <div className="target-picker"><HardDrive size={19} /><div><span>Cible d’analyse</span><strong>{target}</strong></div><button onClick={chooseFolder}>Parcourir</button></div>
      <div className="drive-row">{drives.map(drive => <button className={drive.path === target ? 'drive active' : 'drive'} key={drive.path} onClick={() => setTarget(drive.path)}><HardDrive size={16} />{drive.label}</button>)}</div>
      {error && <p className="error-message">{error}</p>}<button className="primary-cta" onClick={start}>Analyser cet emplacement <ChevronRight size={18} /></button>
      <span className="privacy-note">Lecture seule · Analyse locale · Annulable à tout moment</span>
    </section>
  </main>

  if (phase === 'scanning') return <main className="scan-shell"><section className="scan-card">
    <div className="scanning-orb"><LoaderCircle size={34} /></div><span className="eyebrow">ANALYSE EN COURS</span><h1>Cartographie de votre espace</h1>
    <p className="scan-path">{progress?.current_path || target}</p><div className="scan-line"><i /></div>
    <div className="scan-stats"><div><strong>{(progress?.entries || 0).toLocaleString('fr-FR')}</strong><span>éléments lus</span></div><div><strong>{(progress?.directories || 0).toLocaleString('fr-FR')}</strong><span>dossiers</span></div><div><strong>{elapsed(progress?.elapsed_ms || 0)}</strong><span>écoulées</span></div></div>
    <button className="quiet-button" onClick={cancel}><X size={16} /> Annuler l’analyse</button>
  </section></main>

  return <main className="app-shell">
    <aside className="sidebar"><div className="side-brand"><div className="brand-mark small"><Sparkles size={16} /></div><span>WinSpace</span></div><button className="new-scan" onClick={() => setPhase('home')}>+ Nouvelle analyse</button>
      <p className="side-label">EMPLACEMENTS</p>{drives.map(drive => <button className="side-location" key={drive.path} onClick={() => { setTarget(drive.path); setPhase('home') }}><HardDrive size={16}/><span>{drive.label}</span></button>)}
      <p className="side-label">RÉCENT</p><button className="side-location selected"><HardDrive size={16}/><span>{root?.path || target}</span></button><div className="side-footer"><button className="side-location"><Settings2 size={16}/><span>Préférences</span></button></div>
    </aside>
    <section className="workspace"><header className="workspace-header"><div><span className="eyebrow">{isScanning ? 'ANALYSE EN DIRECT' : 'ANALYSE TERMINÉE'}</span><div className="crumbs"><button onClick={back} disabled={isScanning || history.length === 0} title="Revenir au dossier précédent"><ArrowLeft size={17}/></button><span>{root?.name || root?.path}</span>{current && current.path !== root?.path && <><ChevronRight size={16}/><strong>{current.name}</strong></>}</div></div><div className="header-actions"><label><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="Rechercher"/></label><button className="icon-button" title="Réglages"><Settings2 size={18}/></button></div></header>
      <div className="overview"><div><span className="label">{isScanning ? 'ESPACE DÉJÀ MESURÉ' : 'ESPACE ANALYSÉ'}</span><strong>{formatSize(root?.size || total)}</strong><small>{items.length} éléments au niveau actuel</small></div><div className="overview-track"><div className={isScanning ? 'live-progress' : ''} style={{ width: isScanning ? '58%' : '74%' }} /><span>{isScanning ? `${(progress?.entries || 0).toLocaleString('fr-FR')} éléments parcourus · résultats provisoires` : 'Analyse locale terminée'}</span></div>{isScanning ? <button className="cancel-live" onClick={cancel}><X size={14}/> Arrêter</button> : <button className="sort-toggle" onClick={() => setSort(sort === 'size' ? 'name' : 'size')}>{sort === 'size' ? 'Taille ↓' : 'Nom A–Z'}</button>}</div>
      {(isScanning || capped) && <div className="live-notice"><span><i />{isScanning ? 'Première carte disponible : les tailles se précisent pendant le scan.' : 'Index de navigation protégé : les résultats principaux sont complets.'}</span>{isScanning && <span>Navigation détaillée disponible à la fin</span>}</div>}
      {categories.length > 0 && <section className="category-card"><div className="section-title"><div><span className="label">RÉPARTITION PAR TYPE</span><h2>Ce qui occupe l’espace</h2></div><span>{isScanning ? 'Données provisoires' : 'Tous les fichiers classés'}</span></div><div className="category-grid">{categories.slice(0, 7).map(category => { const Icon = categoryIcon(category.id); const share = root?.size ? Math.min(100, (category.size / root.size) * 100) : 0; return <div className="category-item" key={category.id}><span className={`category-icon ${category.id}`}><Icon size={16}/></span><div><strong>{category.label}</strong><small>{formatSize(category.size)} · {category.file_count.toLocaleString('fr-FR')} fichiers</small><i><b style={{ width: `${share}%` }} /></i></div><em>{share.toFixed(0)} %</em></div> })}</div></section>}
      <section className="visual-card"><div className="section-title"><div><span className="label">CARTE D’ESPACE</span><h2>{current?.name || current?.path}</h2></div><span>{isScanning ? 'Mise à jour en direct' : 'Double-cliquer pour explorer'}</span></div><div className="bubble-map">{filtered.slice(0, 18).map((node, index) => { const ratio = total ? node.size / total : 0; const diameter = Math.max(72, Math.min(238, 75 + Math.sqrt(ratio) * 285)); return <button key={node.path} className={`bubble ${selected?.path === node.path ? 'chosen' : ''}`} style={{ width: diameter, height: diameter, background: `radial-gradient(circle at 30% 25%, ${colorFor(node.name, index)}dd, ${colorFor(node.name, index)}52)` }} onClick={() => setSelected(node)} onDoubleClick={() => void openNode(node)}><span>{node.name}</span><small>{formatSize(node.size)}</small>{isScanning && <b className="bubble-live"><i />Calcul</b>}</button> })}{!filtered.length && <p className="empty-state">Aucun élément ne correspond à la recherche.</p>}</div></section>
      <section className="table-card"><div className="section-title"><div><span className="label">DÉTAILS</span><h2>Éléments de ce dossier</h2></div><span>{filtered.length} éléments</span></div><div className="table-head"><span>Nom</span><span>Taille</span><span>Part</span><span /></div>{filtered.slice(0, 80).map((node, index) => <button className={`table-row ${selected?.path === node.path ? 'chosen' : ''}`} key={node.path} onClick={() => setSelected(node)} onDoubleClick={() => void openNode(node)}><span><i style={{ background: colorFor(node.name, index) }} />{node.name}{priorityFor(node.name) && <em className="priority">{priorityFor(node.name)}</em>}{node.inaccessible && <em>Accès limité</em>}</span><strong>{formatSize(node.size)}</strong><span>{total ? `${((node.size / total) * 100).toFixed(1)} %` : '—'}</span><ChevronRight size={17}/></button>)}</section>
    </section>
    <aside className="inspector">{selected ? <><div className="inspector-top"><span className="label">SÉLECTION</span><button className="icon-button" onClick={() => setSelected(null)}><X size={16}/></button></div><div className="selection-icon" style={{ background: colorFor(selected.name, 0) }}><FolderOpen size={28}/></div><h2>{selected.name}</h2><p className="path-text">{selected.is_virtual ? 'Fichiers directement présents dans ce dossier' : selected.path}</p><div className="detail-grid"><div><span>Taille</span><strong>{formatSize(selected.size)}</strong></div><div><span>Part</span><strong>{total ? `${((selected.size / total) * 100).toFixed(1)} %` : '—'}</strong></div><div><span>Contenu</span><strong>{selected.child_count.toLocaleString('fr-FR')} éléments</strong></div><div><span>État</span><strong>{selected.inaccessible ? 'Partiel' : isScanning ? 'En cours' : 'Disponible'}</strong></div></div>{selected.is_dir && !isScanning && <button className="primary-cta compact" onClick={() => void openNode(selected)}>Explorer ce dossier <ChevronRight size={17}/></button>}{!selected.is_virtual && <button className="outline-button" onClick={reveal}><FolderOpen size={16}/> Ouvrir dans Explorer</button>}<button className="outline-button" onClick={copyPath}><Copy size={16}/> Copier le chemin</button></> : <div className="nothing-selected"><div className="selection-icon muted"><Sparkles size={25}/></div><h3>Explorer votre espace</h3><p>Sélectionnez une bulle ou une ligne pour voir ses détails.</p></div>}</aside>
  </main>
}
