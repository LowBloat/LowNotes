<script lang="ts">
  import { Bold, Code, Heading1, Heading2, Italic, List, ListTodo, LoaderCircle, MessageSquare, Network, Quote, Strikethrough } from 'lucide-svelte';
  import { onMount, onDestroy, tick } from 'svelte';
  import { EditorView, basicSetup } from 'codemirror';
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
  import { defaultHighlightStyle, HighlightStyle, syntaxHighlighting } from '@codemirror/language';
  import { styleTags, tags } from '@lezer/highlight';
  import { Compartment, EditorState } from '@codemirror/state';
  import { search, SearchQuery, setSearchQuery, replaceNext, replaceAll } from '@codemirror/search';
  import { clearPreviewSearch, combineSearchMatches, highlightPreviewSearch, noteSearchHighlights, previewSearchMatches, previewSourceTarget, searchMatches, setNoteSearchHighlights, type NoteSearchMatch } from '$lib/note-search';
  import NoteFindPanel from './NoteFindPanel.svelte';
  import * as Y from 'yjs';
  import { createLocalCollaboration } from '$lib/editor-collaboration';
  import { openEditorLinks } from '$lib/editor-links';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { createImagePaste, type ImagePasteStatus } from '$lib/image-paste';
  import { renderMermaidSvg } from '$lib/mermaid-renderer';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { crdtApplyClientUpdate, readNote, broadcastAwareness, uploadClipboardImage, localImageUrl } from '../api';
  import { renderMarkdown } from '../markdown';
  import { taskCheckboxChange } from '$lib/markdown-tasks';
  import DocumentActions from './DocumentActions.svelte';
  import type { AppTheme, ViewMode, ImageUploadProvider } from '../types';
  import { t, ts, trError } from '$lib/i18n';
  import {
    Awareness,
    applyAwarenessUpdate,
    encodeAwarenessUpdate,
    outdatedTimeout,
    removeAwarenessStates,
  } from 'y-protocols/awareness';
  import { colorForDevice, parsePresenceState, type PresenceUser } from '$lib/presence';

  let {
    notePath,
    initialContent,
    crdtUpdateBase64,
    targetLine,
    theme,
    viewMode,
    lineWrapping = true,
    imageUploadProvider = 'local',
    vaultId = '',
    onViewModeChange,
    isAiChatOpen = false,
    onToggleAiChat,
    onContentChange,
    onLocalEdit,
    onOpenNote,
    onOpenGraph,
    onOpenWikilink,
    deviceName = '',
    deviceId = '',
  } = $props<{
    notePath: string;
    initialContent: string;
    crdtUpdateBase64?: string;
    targetLine?: number;
    theme: AppTheme;
    viewMode: ViewMode;
    lineWrapping?: boolean;
    imageUploadProvider?: ImageUploadProvider;
    vaultId?: string;
    onViewModeChange: (mode: ViewMode) => void;
    isAiChatOpen?: boolean;
    onToggleAiChat?: () => void;
    onContentChange?: (path: string, newContent: string) => void;
    onLocalEdit?: () => void;
    onOpenNote?: (path: string) => void;
    onOpenGraph?: () => void;
    onOpenWikilink?: (title: string, kind?: 'wiki' | 'markdown') => void;
    deviceName?: string;
    deviceId?: string;
  }>();

  let editorContainer: HTMLDivElement | null = $state(null);
  let editorRoot: HTMLDivElement | null = $state(null);
  let previewContainer: HTMLDivElement | null = $state(null);
  let findPanel: NoteFindPanel | undefined = $state();
  let findOpen = $state(false);
  let findText = $state('');
  let findReplacement = $state('');
  let findCase = $state(false);
  let findRegex = $state(false);
  let findWord = $state(false);
  let findReplacing = $state(false);
  let findResults = $state<NoteSearchMatch[]>([]);
  let findCurrent = $state(0);
  let findAnchor = 0;
  let findSignature = '';
  let findMode: ViewMode | undefined;
  let findWasOpen = false;
  let findNeedsScroll = false;
  let findReturnFocus: HTMLElement | null = null;
  let findRefresh = 0;
  let previewRenderRevision = $state(0);
  const findInvalid = $derived(findRegex && !!findText && !new SearchQuery({ search: findText, regexp: true }).valid);
  let saveStatus = $state<'saved' | 'error'>('saved');
  let linkOpenFailed = $state(false);
  let currentContent = $state('');
  let contentRevision = $state(0);
  let wordCount = $derived(
    currentContent.trim() ? currentContent.trim().split(/\s+/).length : 0
  );
  let charCount = $derived(currentContent.length);
  let imageUploads = $state<ImagePasteStatus[]>([]);
  let imageRevision = $state(0);
  let unlistenImages: UnlistenFn | null = null;
  let imagePaste: ReturnType<typeof createImagePaste> | null = null;

  let editorView: EditorView | null = null;
  let yDoc: Y.Doc | null = null;
  let undoManager: Y.UndoManager | null = null;
  let unlistenCrdt: UnlistenFn | null = null;
  let awareness: Awareness | null = null;
  let remoteUsers = $state<PresenceUser[]>([]);
  let awarenessSendTimer: ReturnType<typeof setTimeout> | null = null;
  let stalePruneTimer: ReturnType<typeof setInterval> | undefined;
  let unlistenAwareness: UnlistenFn | null = null;
  let pendingRemoteUpdates: Uint8Array[] = [];
  let disposed = false;
  let mermaidCounter = 0;
  let mermaidDebounce: ReturnType<typeof setTimeout> | null = null;
  const editorTheme = new Compartment();
  const editorWrapping = new Compartment();

  function findQuery() {
    return new SearchQuery({ search: findText, replace: findReplacement, caseSensitive: findCase, regexp: findRegex, wholeWord: findWord });
  }

  async function openFind(replacing = false) {
    if (!findOpen) {
      findReturnFocus = document.activeElement instanceof HTMLElement && editorRoot?.contains(document.activeElement)
        ? document.activeElement : null;
      const selection = viewMode === 'preview' ? window.getSelection()?.toString() : editorView?.state.sliceDoc(editorView.state.selection.main.from, editorView.state.selection.main.to);
      if (selection && selection.length <= 200 && !selection.includes('\n')) findText = selection;
    }
    if (replacing) findReplacing = true;
    findOpen = true;
    await tick();
    findPanel?.focus();
  }

  function closeFind() {
    findOpen = false;
    if (viewMode === 'preview') previewContainer?.querySelector<HTMLElement>('article')?.focus({ preventScroll: true });
    else if (findReturnFocus?.isConnected) findReturnFocus.focus({ preventScroll: true });
    else editorView?.focus();
  }

  function handleFindShortcut(event: KeyboardEvent) {
    if (event.isComposing || !editorRoot || document.querySelector('[data-modal-backdrop]')) return;
    const modified = (event.ctrlKey || event.metaKey) && !event.altKey;
    const key = event.key.toLowerCase();
    if (modified && (key === 'f' || key === 'h')) {
      event.preventDefault(); event.stopImmediatePropagation();
      void openFind(key === 'h');
    } else if ((modified && key === 'g') || event.key === 'F3') {
      event.preventDefault(); event.stopImmediatePropagation();
      if (findOpen) navigateFind(event.shiftKey ? -1 : 1);
      else void openFind().then(() => navigateFind(event.shiftKey ? -1 : 1));
    } else if (event.key === 'Escape' && findOpen && (editorRoot.contains(event.target as Node) || event.target === document.body)) {
      event.preventDefault(); event.stopImmediatePropagation(); closeFind();
    }
  }

  function showFindResult(scroll: boolean) {
    if (!editorView) return;
    const current = findResults[findCurrent];
    const effects = [setNoteSearchHighlights.of({ matches: findResults, active: findCurrent })];
    if (scroll && current && viewMode !== 'preview') {
      editorView.dispatch({ effects: [...effects, EditorView.scrollIntoView(current.from, { y: 'center', yMargin: 100 })], selection: { anchor: current.from, head: current.to }, userEvent: 'select.search' });
    } else editorView.dispatch({ effects });
    if (!previewContainer) return;
    for (const mark of previewContainer.querySelectorAll<HTMLElement>('mark[data-note-search]')) {
      mark.classList.toggle('note-search-current', Number(mark.dataset.noteSearch) === current?.previewIndex);
    }
    if (!scroll || !current || viewMode === 'edit') return;
    const target = current.previewIndex >= 0
      ? previewContainer.querySelector<HTMLElement>(`mark[data-note-search="${current.previewIndex}"]`) : previewSourceTarget(previewContainer, current.from);
    if (target) {
      const rect = target.getBoundingClientRect(), container = previewContainer.getBoundingClientRect();
      previewContainer.scrollTop += rect.top - container.top - previewContainer.clientHeight / 2 + rect.height / 2;
    }
  }

  function navigateFind(direction: number) {
    if (!findResults.length) return;
    findCurrent = (findCurrent + direction + findResults.length) % findResults.length;
    findAnchor = findResults[findCurrent].from;
    showFindResult(true);
  }

  function refreshFind() {
    if (!editorView) return;
    if (previewContainer) clearPreviewSearch(previewContainer);
    const query = findOpen ? findQuery() : new SearchQuery({ search: '' });
    editorView.dispatch({ effects: setSearchQuery.of(query) });
    const source = searchMatches(editorView.state.doc, query);
    const preview = previewContainer && viewMode !== 'edit' ? previewSearchMatches(previewContainer, editorView.state.doc.length, query) : [];
    findResults = combineSearchMatches(editorView.state, source, preview, viewMode);
    const signature = JSON.stringify([findText, findCase, findRegex, findWord]);
    const navigate = signature !== findSignature || viewMode !== findMode || findOpen !== findWasOpen || findNeedsScroll;
    if (signature !== findSignature) { findAnchor = 0; findSignature = signature; }
    findMode = viewMode;
    findWasOpen = findOpen;
    findNeedsScroll = false;
    const next = findResults.findIndex(match => match.from >= findAnchor);
    findCurrent = next >= 0 ? next : 0;
    if (previewContainer) highlightPreviewSearch(preview, findResults[findCurrent]?.previewIndex ?? -1);
    showFindResult(findOpen && query.valid && navigate);
  }

  function replaceFind(all: boolean) {
    if (!editorView || viewMode === 'preview' || !findQuery().valid) return;
    const current = findResults[findCurrent];
    if (all && !findResults.some(match => match.sourceIndex >= 0)) return;
    if (!all && (!current || current.sourceIndex < 0)) return;
    editorView.dispatch({ effects: setSearchQuery.of(findQuery()), ...(!all && current ? { selection: { anchor: current.from, head: current.to } } : {}) });
    undoManager?.stopCapturing();
    if (all) replaceAll(editorView);
    else replaceNext(editorView);
    undoManager?.stopCapturing();
    findAnchor = editorView.state.selection.main.from;
    findNeedsScroll = true;
  }

  function codeMirrorTheme() {
    return EditorView.theme({
      '&': { height: '100%', outline: 'none' },
      '.cm-scroller': { overflow: 'auto' },
    }, { dark: theme === 'dark' });
  }

  async function renderMermaidBlocks(activeTheme: AppTheme) {
    if (!previewContainer) return;
    const blocks = previewContainer.querySelectorAll<HTMLDivElement>('.mermaid-block');
    if (blocks.length === 0) return;

    const config = {
      startOnLoad: false,
      theme: activeTheme === 'dark' ? 'dark' as const : 'default' as const,
      ...(activeTheme === 'dark' ? {
        themeVariables: {
          darkMode: true,
          background: '#151b26',
          primaryColor: '#d97706',
          primaryTextColor: '#f1f5f9',
          primaryBorderColor: '#232d3d',
          lineColor: '#8e9bb0',
          secondaryColor: '#1c2433',
          tertiaryColor: '#0f141c',
        },
      } : {}),
      fontFamily: 'inherit',
      securityLevel: 'strict' as const,
    };

    for (const block of blocks) {
      const raw = block.getAttribute('data-mermaid');
      if (!raw) continue;
      const code = decodeURIComponent(raw).trim();
      const svgTarget = block.querySelector<HTMLDivElement>('.mermaid-svg');
      if (!svgTarget) continue;

      const id = `mermaid-${Date.now()}-${mermaidCounter++}`;
      try {
        const svg = await renderMermaidSvg(id, code, config);
        svgTarget.innerHTML = svg;
      } catch {
        svgTarget.innerHTML = `<pre class="text-xs text-amber-400/90 font-mono text-left w-full p-2 bg-[var(--bg-main)] rounded border border-amber-900/40 overflow-x-auto whitespace-pre-wrap">${code}</pre>`;
      }
    }
  }

  function base64ToUint8Array(base64: string): Uint8Array {
    let clean = base64.replace(/-/g, '+').replace(/_/g, '/');
    while (clean.length % 4 !== 0) {
      clean += '=';
    }
    const binary = atob(clean);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    return bytes;
  }

  function uint8ArrayToBase64(bytes: Uint8Array): string {
    let binary = '';
    const len = bytes.byteLength;
    for (let i = 0; i < len; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
  }

  function initEditor(content = initialContent, snapshot = crdtUpdateBase64, noteId?: string | null) {
    const editingVaultId = vaultId;
    if (!editorContainer) return;
    imagePaste?.destroy();
    imageUploads = [];

    if (editorView) {
      editorView.destroy();
      editorView = null;
    }
    if (undoManager) {
      undoManager.destroy();
      undoManager = null;
    }
    if (yDoc) {
      yDoc.destroy();
      yDoc = null;
    }

    yDoc = new Y.Doc();
    if (snapshot && snapshot.trim().length > 0) {
      try {
        const update = base64ToUint8Array(snapshot);
        Y.applyUpdate(yDoc, update, 'init');
      } catch (e) {
        console.error('Failed to apply initial CRDT update:', e);
      }
    }

    const yText = yDoc.getText('content');
    if (yText.length === 0 && content.length > 0) {
      yText.insert(0, content);
    }

    currentContent = yText.toString();
    contentRevision++;

    if (awareness) {
      awareness.destroy();
      awareness = null;
    }
    awareness = new Awareness(yDoc);
    awareness.setLocalState({
      user: {
        name: deviceName || 'LowNotes',
        color: colorForDevice(deviceId || 'local'),
        deviceId: deviceId || 'local',
        notePath,
      },
    });
    awareness.on('update', (changes: { added: number[]; updated: number[]; removed: number[] }, origin: unknown) => {
      if (!awareness || origin === 'remote' || origin === 'prune') return;
      const encoded = encodeAwarenessUpdate(awareness, [
        ...changes.added,
        ...changes.updated,
        ...changes.removed,
      ]);
      if (awarenessSendTimer) clearTimeout(awarenessSendTimer);
      awarenessSendTimer = setTimeout(() => {
        broadcastAwareness(notePath, Array.from(encoded)).catch(() => {});
      }, 50);
      refreshRemoteUsers();
    });
    refreshRemoteUsers();

    // Listen to Yjs local edits to broadcast and autosave
    yDoc.on('update', (update: Uint8Array, origin: any) => {
      const text = yText.toString();
      currentContent = text;
      contentRevision++;
      onContentChange?.(notePath, text);

      if (origin !== 'remote') {
        onLocalEdit?.();
        const base64 = uint8ArrayToBase64(update);
        const editingDoc = yDoc;
        crdtApplyClientUpdate(notePath, base64, noteId, editingVaultId)
          .catch(async (error) => {
            if (String(error) === 'errors.noteDeleted' && noteId && editingDoc) {
              try {
                await crdtApplyClientUpdate(notePath, uint8ArrayToBase64(Y.encodeStateAsUpdate(editingDoc)), noteId, editingVaultId, true);
                return;
              } catch (recoveryError) { error = recoveryError; }
            }
            console.error('Failed to persist CRDT update:', error); saveStatus = 'error';
          });
      }
    });

    const collaboration = createLocalCollaboration(yText, awareness);
    undoManager = collaboration.undoManager;
    const uploadVaultId = vaultId;
    imagePaste = createImagePaste({
      upload: (bytes, provider) => uploadClipboardImage(bytes, provider, uploadVaultId),
      getProvider: () => imageUploadProvider,
      onStatus: (statuses) => { imageUploads = statuses; },
      isolateUndo: () => undoManager?.stopCapturing(),
    });
    const state = EditorState.create({
      doc: yText.toString(),
      extensions: [
        basicSetup,
        search(),
        noteSearchHighlights,
        EditorView.updateListener.of(update => { if (update.docChanged) findAnchor = update.changes.mapPos(findAnchor); }),
        markdown({ extensions: [{ props: [styleTags({ 'FencedCode/CodeInfo': tags.special(tags.labelName) })] }] }),
        // Keep standard syntax colors, with palette-aware Markdown links and fence info.
        syntaxHighlighting(defaultHighlightStyle),
        syntaxHighlighting(HighlightStyle.define([
          { tag: [tags.link, tags.url], class: 'cm-readable-link' },
          { tag: tags.special(tags.labelName), class: 'cm-code-info' },
        ], { scope: markdownLanguage })),
        collaboration.extension,
        imagePaste.extension,
        openEditorLinks(async (url) => { linkOpenFailed = false; await openUrl(url); }, () => { linkOpenFailed = true; }),
        editorTheme.of(codeMirrorTheme()),
        editorWrapping.of(lineWrapping ? EditorView.lineWrapping : []),
      ],
    });

    editorView = new EditorView({
      state,
      parent: editorContainer,
    });
  }

  function navigateToLine(lineNumber: number) {
    if (!editorView || lineNumber <= 0) return;
    try {
      const totalLines = editorView.state.doc.lines;
      const target = Math.min(lineNumber, totalLines);
      const line = editorView.state.doc.line(target);
      editorView.dispatch({
        selection: { anchor: line.from },
        scrollIntoView: true,
      });
      editorView.focus();
    } catch (e) {
      console.error('Failed to navigate to line:', e);
    }
  }

  $effect(() => {
    if (targetLine && targetLine > 0 && editorView) {
      navigateToLine(targetLine);
    }
  });

  function applyFormatting(prefix: string, suffix: string = '') {
    if (!editorView) return;
    const { from, to } = editorView.state.selection.main;
    const selectedText = editorView.state.sliceDoc(from, to);
    const replacement = `${prefix}${selectedText || ts('editor.placeholderText')}${suffix}`;

    editorView.dispatch({
      changes: { from, to, insert: replacement },
      selection: { anchor: from + prefix.length, head: from + replacement.length - suffix.length },
    });
    editorView.focus();
  }

  function handlePreviewClick(e: MouseEvent) {
    const target = e.target as Element | null;
    const link = target?.closest('a');
    if (!link) return;
    const wikilink = link.getAttribute('data-wikilink');
    const href = link.getAttribute('href') ?? '';
    if (wikilink !== null || (!/^[a-z][\w+.-]*:/i.test(href) && /\.(?:md|markdown)(?:[?#].*)?$/i.test(href))) {
      e.preventDefault();
      onOpenWikilink?.(wikilink ?? href, wikilink !== null ? 'wiki' : 'markdown');
    } else if (/^https?:\/\//i.test(href)) {
      e.preventDefault();
      linkOpenFailed = false;
      void openUrl(href).catch(() => { linkOpenFailed = true; });
    }
  }

  async function handleTaskChange(event: Event) {
    const input = event.target;
    if (!(input instanceof HTMLInputElement) || !input.matches('input[data-task-offset]') || !editorView) return;
    const wasChecked = input.dataset.taskChecked === 'true';
    const offset = Number(input.dataset.taskOffset);
    const change = input.dataset.taskRevision === String(contentRevision)
      ? taskCheckboxChange(editorView.state.doc.toString(), offset, wasChecked, input.checked) : null;
    if (!change) { input.checked = wasChecked; return; }
    const restoreFocus = document.activeElement === input;
    undoManager?.stopCapturing();
    editorView.dispatch({ changes: change, userEvent: 'input.task' });
    undoManager?.stopCapturing();
    await restoreTaskFocus(offset, restoreFocus);
  }

  async function restoreTaskFocus(offset: number, restoreFocus: boolean) {
    await tick();
    if (restoreFocus && (!document.activeElement || document.activeElement === document.body)) {
      previewContainer?.querySelector<HTMLInputElement>(`input[data-task-offset="${offset}"]`)?.focus({ preventScroll: true });
    }
  }

  async function handleTaskUndo(event: KeyboardEvent) {
    if (!(event.target instanceof HTMLInputElement) || !event.target.matches('input[data-task-offset]')
      || !undoManager || !(event.ctrlKey || event.metaKey) || event.altKey) return;
    const key = event.key.toLowerCase();
    if (key !== 'z' && key !== 'y') return;
    event.preventDefault();
    const offset = Number(event.target.dataset.taskOffset);
    if (key === 'y' || event.shiftKey) undoManager.redo();
    else undoManager.undo();
    await restoreTaskFocus(offset, true);
  }

  function refreshRemoteUsers() {
    if (!awareness) {
      remoteUsers = [];
      return;
    }
    const users: PresenceUser[] = [];
    for (const [clientId, state] of awareness.getStates()) {
      if (clientId === awareness.clientID) continue;
      const user = parsePresenceState(state)?.user;
      if (user && user.notePath === notePath && user.deviceId) {
        users.push(user);
      }
    }
    remoteUsers = users;
  }

  function pruneStalePeers() {
    if (!awareness) return;
    const now = Date.now();
    const stale: number[] = [];
    awareness.meta.forEach((meta, clientId) => {
      if (clientId !== awareness!.clientID && now - meta.lastUpdated > outdatedTimeout) {
        stale.push(clientId);
      }
    });
    if (stale.length > 0) {
      removeAwarenessStates(awareness, stale, 'prune');
      refreshRemoteUsers();
    }
  }

  onMount(async () => {
    window.addEventListener('keydown', handleFindShortcut, true);
    const stopCrdt = await listen<{ note_path: string; update: number[]; vault_id?: string }>(
      'p2p:crdt-update',
      (event) => {
        if (event.payload.note_path === notePath && (!event.payload.vault_id || event.payload.vault_id === vaultId)) {
          const update = new Uint8Array(event.payload.update);
          if (yDoc) Y.applyUpdate(yDoc, update, 'remote');
          else pendingRemoteUpdates.push(update);
        }
      }
    );
    if (disposed) { stopCrdt(); return; }
    unlistenCrdt = stopCrdt;

    const stopAwareness = await listen<{ note_path: string; update: number[] }>(
      'p2p:awareness',
      (event) => {
        if (event.payload.note_path === notePath && awareness) {
          applyAwarenessUpdate(awareness, new Uint8Array(event.payload.update), 'remote');
          refreshRemoteUsers();
        }
      }
    );
    if (disposed) { stopAwareness(); return; }
    unlistenAwareness = stopAwareness;
    const stopImages = await listen('p2p:synced', () => { imageRevision++; });
    if (disposed) { stopImages(); return; }
    unlistenImages = stopImages;

    // Read after listeners are ready so edits arriving while this note opens are not lost.
    try {
      const latest = await readNote(notePath);
      if (disposed) return;
      initEditor(latest.content, latest.crdt_update_base64, latest.note_id);
    } catch (error) {
      if (disposed) return;
      console.error('Failed to refresh note before editing:', error);
      initEditor();
    }
    for (const update of pendingRemoteUpdates) {
      if (yDoc) Y.applyUpdate(yDoc, update, 'remote');
    }
    pendingRemoteUpdates = [];

    stalePruneTimer = setInterval(pruneStalePeers, 15000);
  });

  onDestroy(() => {
    window.removeEventListener('keydown', handleFindShortcut, true);
    findRefresh++;
    imagePaste?.destroy();
    disposed = true;
    if (editorView) editorView.destroy();
    if (undoManager) undoManager.destroy();
    if (yDoc) yDoc.destroy();
    if (unlistenCrdt) unlistenCrdt();
    if (unlistenAwareness) unlistenAwareness();
    if (unlistenImages) unlistenImages();
    if (awarenessSendTimer) clearTimeout(awarenessSendTimer);
    if (stalePruneTimer) clearInterval(stalePruneTimer);
    if (awareness) {
      awareness.destroy();
      awareness = null;
    }
  });
  $effect(() => {
    if (editorView) {
      editorView.dispatch({ effects: editorTheme.reconfigure(codeMirrorTheme()) });
    }
  });

  $effect(() => {
    currentContent; contentRevision; viewMode; previewContainer; findOpen; findText; findCase; findRegex; findWord; imageRevision; theme; previewRenderRevision;
    const revision = ++findRefresh;
    void tick().then(() => { if (!disposed && revision === findRefresh) refreshFind(); });
  });

  $effect(() => {
    const wrap = lineWrapping;
    if (editorView) {
      editorView.dispatch({ effects: editorWrapping.reconfigure(wrap ? EditorView.lineWrapping : []) });
    }
  });

  $effect(() => {
    if ((viewMode === 'split' || viewMode === 'preview') && previewContainer && currentContent) {
      imageRevision;
      const activeTheme = theme;
      if (mermaidDebounce) clearTimeout(mermaidDebounce);
      mermaidDebounce = setTimeout(() => {
        void renderMermaidBlocks(activeTheme).then(() => { if (!disposed) previewRenderRevision++; });
      }, 60);
    }
  });
</script>

<div bind:this={editorRoot} class="flex flex-col min-w-0 min-h-0 h-full w-full bg-[var(--bg-main)]">
  <!-- Top Editor Toolbar -->
  <header class="app-topbar flex items-center gap-3 overflow-x-auto whitespace-nowrap px-4 border-b border-[var(--border)] bg-[var(--bg-sidebar)] select-none">
    <div class="flex shrink-0 items-center gap-1">
      <button
        onclick={() => applyFormatting('**', '**')}
        class="px-2 py-1 text-xs font-bold rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('editor.bold')}
        aria-label={$t('editor.bold')}
      >
        <Bold size={14} />
      </button>
      <button
        onclick={() => applyFormatting('*', '*')}
        class="px-2 py-1 text-xs italic rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('editor.italic')}
        aria-label={$t('editor.italic')}
      >
        <Italic size={14} />
      </button>
      <button
        onclick={() => applyFormatting('~~', '~~')}
        class="px-2 py-1 text-xs line-through rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('editor.strikethrough')}
        aria-label={$t('editor.strikethrough')}
      >
        <Strikethrough size={14} />
      </button>
      <span class="w-[1px] h-4 bg-[var(--border)] mx-1"></span>
      <button
        onclick={() => applyFormatting('# ')}
        class="px-2 py-1 text-xs font-semibold rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('editor.heading1')}
        aria-label={$t('editor.heading1')}
      >
        <Heading1 size={16} />
      </button>
      <button
        onclick={() => applyFormatting('## ')}
        class="px-2 py-1 text-xs font-semibold rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('editor.heading2')}
        aria-label={$t('editor.heading2')}
      >
        <Heading2 size={16} />
      </button>
      <button
        onclick={() => applyFormatting('- ')}
        class="px-2 py-1 text-xs rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition inline-flex items-center gap-1.5"
        title={$t('editor.list')}
      >
        <List size={14} /> {$t('editor.list')}
      </button>
      <button
        onclick={() => applyFormatting('- [ ] ')}
        class="px-2 py-1 text-xs rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition inline-flex items-center gap-1.5"
        title={$t('editor.checklist')}
      >
        <ListTodo size={14} /> {$t('editor.task')}
      </button>
      <button
        onclick={() => applyFormatting('`', '`')}
        class="px-2 py-1 text-xs font-mono rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('editor.code')}
        aria-label={$t('editor.code')}
      >
        <Code size={16} />
      </button>
      <button
        onclick={() => applyFormatting('> ')}
        class="px-2 py-1 text-xs rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition inline-flex items-center gap-1.5"
        title={$t('editor.quote')}
      >
        <Quote size={14} /> {$t('editor.quote')}
      </button>
      <span class="w-[1px] h-4 bg-[var(--border)] mx-1"></span>
      <button
        onclick={() => onOpenGraph?.()}
        class="px-2 py-1 text-xs rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition inline-flex items-center gap-1.5"
        title={$t('graph.toolbarTitle')}
      >
        <Network size={14} /><span>{$t('graph.button')}</span>
      </button>
    </div>

    <!-- Mode Selector & Status -->
    <div class="flex shrink-0 items-center gap-3 ml-auto">
      {#if remoteUsers.length > 0}
        <div
          class="flex items-center -space-x-1.5"
          title={remoteUsers.map((u) => u.name).join(', ')}
        >
          {#each remoteUsers as user (user.deviceId)}
            <span
              class="w-5 h-5 rounded-full border-2 border-[var(--bg-sidebar)] flex items-center justify-center text-[9px] font-bold text-black select-none"
              style="background: {user.color}"
            >
              {user.name.slice(0, 1).toUpperCase()}
            </span>
          {/each}
        </div>
      {/if}
      <div class="flex items-center gap-1.5 text-xs text-[var(--text-dim)]">
        {#if saveStatus === 'error'}
          <span class="text-[var(--danger)]">{$t('editor.saveError')}</span>
        {:else}
          <span class="inline-block w-2 h-2 rounded-full bg-[var(--success)]"></span>
          <span>{$t('editor.saved')}</span>
        {/if}
      </div>

      <div class="flex items-center bg-[var(--bg-card)] p-0.5 rounded-md border border-[var(--border)]">
        <button
          onclick={() => onViewModeChange('edit')}
          class="px-2.5 py-1 text-xs rounded transition {viewMode === 'edit' ? 'bg-[var(--bg-active)] text-[var(--accent-light)] font-medium shadow-sm' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}"
        >
          {$t('editor.modeEdit')}
        </button>
        <button
          onclick={() => onViewModeChange('split')}
          class="px-2.5 py-1 text-xs rounded transition {viewMode === 'split' ? 'bg-[var(--bg-active)] text-[var(--accent-light)] font-medium shadow-sm' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}"
        >
          {$t('editor.modeSplit')}
        </button>
        <button
          onclick={() => onViewModeChange('preview')}
          class="px-2.5 py-1 text-xs rounded transition {viewMode === 'preview' ? 'bg-[var(--bg-active)] text-[var(--accent-light)] font-medium shadow-sm' : 'text-[var(--text-muted)] hover:text-[var(--text-main)]'}"
        >
          {$t('editor.modePreview')}
        </button>
      </div>

      <DocumentActions content={currentContent} path={notePath} />

      {#if onToggleAiChat}
        <button
          onclick={onToggleAiChat}
          class="px-2.5 py-1 text-xs rounded transition flex items-center gap-1.5 border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--accent-light)] {isAiChatOpen ? 'border-[var(--accent)] text-[var(--accent-light)] font-medium shadow-sm bg-[var(--bg-active)]' : ''}"
          title={$t('editor.openAiAssistant')}
        >
          <MessageSquare size={15} />
          <span class="font-medium">{$t('editor.assistant')}</span>
        </button>
      {/if}
    </div>
  </header>

  <!-- Editor & Preview Body -->
  {#if linkOpenFailed}
    <div role="alert" class="flex items-center gap-2 border-b border-[var(--border)] bg-[var(--bg-card)] px-4 py-2 text-xs text-[var(--danger)]">
      {$t('editor.linkOpenError')}
      <button class="ml-auto underline text-[var(--text-muted)]" onclick={() => { linkOpenFailed = false; }}>{$t('ai.close')}</button>
    </div>
  {/if}
  {#if imageUploads.length}
    <div class="border-b border-[var(--border)] bg-[var(--bg-card)] px-4 py-2 text-xs space-y-2">
      {#each imageUploads as upload (upload.id)}
        <div class="flex flex-wrap items-center gap-2">
          {#if upload.status === 'uploading'}
            <span role="status" class="flex items-center gap-2 text-[var(--accent-light)]">
              <LoaderCircle size={15} class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              {$t(upload.provider === 'local' ? 'editor.imageSavingLocal' : 'editor.imageUploading', { provider: upload.provider === 'imgur' ? 'Imgur' : 'Catbox', name: upload.name, uploaded: upload.uploaded, count: upload.count })}
            </span>
          {:else if upload.status === 'error'}
            <span role="alert" class="text-[var(--danger)]">{upload.name}: {trError(upload.error)}</span>
            <button class="underline text-[var(--accent-light)]" onclick={() => imagePaste?.retry(upload.id)}>{$t('editor.imageRetry')}</button>
          {:else}
            <span role="status">{$t('editor.imagePositionDeleted')}</span>
            <button class="underline text-[var(--accent-light)]" onclick={() => imagePaste?.insertAtCursor(upload.id)}>{$t('editor.imageInsertHere')}</button>
          {/if}
          <button class="ml-auto shrink-0 underline text-[var(--text-muted)]" onclick={() => imagePaste?.cancel(upload.id)}>{$t('editor.imageCancel')}</button>
        </div>
      {/each}
    </div>
  {/if}
  <main class="flex-1 flex min-w-0 min-h-0 overflow-hidden relative">
    {#if findOpen}
      <NoteFindPanel bind:this={findPanel} bind:query={findText} bind:replacement={findReplacement} bind:caseSensitive={findCase} bind:regexp={findRegex} bind:wholeWord={findWord} bind:replacing={findReplacing}
        mode={viewMode} count={findResults.length} current={findCurrent} invalid={findInvalid}
        canReplace={findResults[findCurrent]?.sourceIndex >= 0} canReplaceAll={findResults.some(match => match.sourceIndex >= 0)}
        onNavigate={navigateFind} onClose={closeFind} onReplace={replaceFind} />
    {/if}
    <!-- CodeMirror Container -->
    <div
      bind:this={editorContainer}
      class="min-w-0 h-full overflow-hidden transition-all duration-150 {viewMode === 'edit' ? 'w-full' : viewMode === 'split' ? 'w-1/2 border-r border-[var(--border)]' : 'hidden'}"
    ></div>

    <!-- Rendered Markdown Container -->
    {#if viewMode === 'split' || viewMode === 'preview'}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        bind:this={previewContainer}
        role="presentation"
        onclick={handlePreviewClick}
        onchange={handleTaskChange}
        onkeydown={handleTaskUndo}
        class="min-w-0 h-full overflow-y-auto px-8 py-6 select-text {viewMode === 'preview' ? 'w-full max-w-4xl mx-auto' : 'w-1/2'}"
      >
        <article tabindex="-1" class="prose max-w-none text-[var(--text-main)]">
          {@html renderMarkdown(currentContent, (src) => localImageUrl(src, vaultId, imageRevision), { interactiveTasks: true, taskRevision: contentRevision, sourceMap: 'editor' })}
        </article>
      </div>
    {/if}
  </main>

  <!-- Status Bar Footer -->
  <footer class="flex items-center justify-between px-4 py-1.5 border-t border-[var(--border)] bg-[var(--bg-sidebar)] text-xs text-[var(--text-dim)] select-none">
    <div class="flex items-center gap-3">
      <span>{notePath}</span>
    </div>
    <div class="flex items-center gap-4">
      <span>{$t('editor.words', { count: wordCount })}</span>
      <span>{$t('editor.chars', { count: charCount })}</span>
      <span class="text-[var(--accent-light)] font-mono">{$t('editor.p2pRealtime')}</span>
    </div>
  </footer>

</div>

<style>
  :global(.prose) {
    line-height: 1.7;
    font-size: 15px;
  }
  :global(.prose h1) {
    font-size: 1.85rem;
    font-weight: 700;
    margin-top: 1.5rem;
    margin-bottom: 0.8rem;
    color: var(--text-main);
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.4rem;
  }
  :global(.prose h2) {
    font-size: 1.4rem;
    font-weight: 600;
    margin-top: 1.3rem;
    margin-bottom: 0.6rem;
    color: var(--text-main);
  }
  :global(.prose h3) {
    font-size: 1.15rem;
    font-weight: 600;
    margin-top: 1rem;
    margin-bottom: 0.4rem;
    color: var(--text-main);
  }
  :global(.prose p) {
    margin-top: 0.6rem;
    margin-bottom: 0.6rem;
  }
  :global(.prose code) {
    background-color: var(--bg-card);
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 13px;
    border: 1px solid var(--border);
  }
  :global(.prose pre) {
    background-color: var(--bg-card);
    padding: 12px 16px;
    border-radius: 6px;
    border: 1px solid var(--border);
    overflow-x: auto;
  }
  :global(.prose pre code) {
    padding: 0;
    border: 0;
    background: transparent;
  }
  :global(.prose blockquote) {
    border-left: 3px solid var(--accent);
    padding-left: 12px;
    color: var(--text-muted);
    font-style: italic;
  }
  :global(.prose ul) {
    list-style-type: disc;
    padding-left: 1.4rem;
    margin: 0.6rem 0;
  }
  :global(.prose ol) {
    list-style-type: decimal;
    padding-left: 1.4rem;
    margin: 0.6rem 0;
  }
  :global(.prose a) {
    color: var(--accent-light);
    text-decoration: underline;
  }
  :global(.wikilink) {
    color: var(--accent-light);
    text-decoration: underline;
    cursor: pointer;
  }
  :global(.prose table) {
    width: 100%;
    border-collapse: collapse;
    margin: 1rem 0;
  }
  :global(.prose th, .prose td) {
    border: 1px solid var(--border);
    padding: 8px 12px;
    text-align: left;
  }
  :global(.prose th) {
    background-color: var(--bg-card);
  }
  :global(.prose dl) {
    margin: 0.8rem 0 1.2rem;
  }
  :global(.prose dt) {
    font-weight: 650;
    margin-top: 0.8rem;
  }
  :global(.prose dd) {
    margin: 0.2rem 0 0.7rem 1.25rem;
    color: var(--text-muted);
  }
  :global(.prose dd p) {
    margin: 0.35rem 0;
  }
  :global(.prose .footnotes) {
    margin-top: 2rem;
    border-top: 1px solid var(--border);
    padding-top: 0.8rem;
    color: var(--text-muted);
    font-size: 0.9em;
  }
  :global(.prose .footnotes-sep) {
    border: 0;
    border-top: 1px solid var(--border);
    margin-top: 2rem;
  }
  :global(.prose .footnote-ref), :global(.prose .footnote-backref) {
    font-size: 0.85em;
  }
  :global(.prose abbr) {
    text-decoration: underline dotted;
    cursor: help;
  }
  :global(.prose mark) {
    background: var(--accent-glow);
    color: var(--text-main);
    border-radius: 2px;
  }
  :global(.prose .warning), :global(.prose .info), :global(.prose .tip), :global(.prose .danger) {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: 6px;
    padding: 0.6rem 1rem;
    margin: 1rem 0;
  }
  :global(.prose .danger) {
    border-left-color: var(--danger);
  }
  :global(.prose .tip) {
    border-left-color: var(--success);
  }
  :global(.prose .mermaid-block) {
    margin: 1rem 0;
    padding: 1rem;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow-x: auto;
    text-align: center;
  }
  :global(.prose .mermaid-svg) {
    display: flex;
    justify-content: center;
  }
  :global(.prose img) {
    max-width: 100%;
  }
  :global(.prose .task-list-item) {
    list-style-type: none;
  }
  :global(.prose .task-list-item input) {
    margin-right: 0.4rem;
    accent-color: var(--accent);
  }
  :global(.prose .task-list-item input:not(:disabled)), :global(.prose .task-list-item label:has(input:not(:disabled))) {
    cursor: pointer;
  }
  :global(.prose .task-list-item input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 3px;
  }
</style>
