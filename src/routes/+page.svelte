<script lang="ts">
  import { FileText, FolderOpen, MessageSquare, Network, NotebookPen } from 'lucide-svelte';
  import { onMount, onDestroy, tick } from 'svelte';
  import type { AppSettings, AppTheme, InitialStateResponse, ViewMode, VaultConfig, VaultItem } from '$lib/types';
  import {
    getAppState,
    listNotes,
    pickVaultDirectory,
    selectVault,
    saveViewMode,
    saveUpdatePrefs,
    takeOpenRequest,
    undoLastDelete,
  } from '$lib/api';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { createP2pSession } from '$lib/state/p2p-session.svelte';
  import { createPresenceTracker } from '$lib/state/presence.svelte';
  import { createUpdateChecker } from '$lib/state/updates.svelte';
  import { createRecoveryNotices } from '$lib/state/recovery.svelte';
  import { locale, resolveLocale, t, trError, ts } from '$lib/i18n';
  import { configureRemoteImages } from '$lib/markdown';
  import { appAlert, appConfirm, dialogState } from '$lib/dialogs.svelte';
  import AppDialog from '$lib/components/AppDialog.svelte';
  import ContextMenu from '$lib/components/ContextMenu.svelte';
  import { setTrayLanguage, windowPainted } from '$lib/api';
  import { resolveNoteLink } from '$lib/note-links';
  import { DEFAULT_PALETTE_ID, applyTheme } from '$lib/themes';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import Editor from '$lib/components/Editor.svelte';
  import GraphView from '$lib/components/GraphView.svelte';
  import PairModal from '$lib/components/PairModal.svelte';
  import IncomingPairDialog from '$lib/components/IncomingPairDialog.svelte';
  import AiChatSidebar from '$lib/components/AiChatSidebar.svelte';
  import WelcomeModal from '$lib/components/WelcomeModal.svelte';
  import UpdateModal from '$lib/components/UpdateModal.svelte';
  import SettingsView from '$lib/components/SettingsView.svelte';
  import HistoryModal from '$lib/components/HistoryModal.svelte';
  let settings = $state<AppSettings | null>(null);
  // Assistant messages follow the same remote image policy as the preview.
  $effect(() => {
    configureRemoteImages({ load: settings?.load_remote_images === true, label: $t('preview.remoteImageBlocked') });
  });
  // The tray menu lives in the native process; keep its labels in the interface language.
  $effect(() => { void setTrayLanguage($locale).catch(() => {}); });
  let theme = $state<AppTheme>('light');
  let viewMode = $state<ViewMode>('split');
  let isGraphOpen = $state(false);
  let activeVault = $state<VaultConfig | null>(null);
  let vaults = $state<VaultConfig[]>([]);
  let items = $state<VaultItem[]>([]);
  let selectedNotePath = $state<string>('');
  let currentNoteContent = $state<string>('');
  let currentCrdtBase64 = $state<string>('');

  // Shell state lives in focused modules; the page wires them together.
  const p2p = createP2pSession({
    onItemsChanged: () => void refreshItems(),
    onPeerApproved: (peer) => {
      if (activeVault && !activeVault.peers.some((p) => p.endpoint_id === peer.endpoint_id)) activeVault.peers.push(peer);
    },
  });
  const presence = createPresenceTracker();
  const updates = createUpdateChecker(() => settings);
  const recovery = createRecoveryNotices();

  let isPairModalOpen = $state(false);
  let isAiChatOpen = $state(false);
  let isSettingsOpen = $state(false);
  let isHistoryOpen = $state(false);
  let settingsTab = $state<'general' | 'themes' | 'ai' | 'providers' | 'web' | 'about'>('general');
  let isWelcomeOpen = $state(false);
  // A new object per request, so navigating twice to the same line works.
  let targetLine = $state<{ line: number; nonce: number } | undefined>(undefined);
  let updateCheckLocal = $state(true);
  let undoPending = false;
  let lastUndoableAction: 'delete' | 'text' = 'text';

  async function handleUndoDeletedItem(event: KeyboardEvent) {
    if (event.key.toLowerCase() !== 'z' || !(event.ctrlKey || event.metaKey)
      || event.shiftKey || event.altKey || event.repeat || event.defaultPrevented || undoPending
      || isSettingsOpen || !activeVault || document.querySelector('[data-modal-backdrop]')) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('.cm-editor')) {
      if (lastUndoableAction !== 'delete') return;
    } else if (target?.closest('input, textarea, select, [contenteditable="true"]')) return;
    event.preventDefault();
    event.stopPropagation();
    undoPending = true;
    try {
      const restored = await undoLastDelete();
      if (restored) {
        await refreshItems();
        if (!restored.is_dir) await openNote(restored.path);
        lastUndoableAction = restored.has_more ? 'delete' : 'text';
      } else lastUndoableAction = 'text';
    } catch (error) {
      void appAlert(trError(String(error)));
    } finally {
      undoPending = false;
    }
  }
  async function loadInitialData() {
    try {
      const data = await getAppState();
      settings = data.settings;
      locale.set(resolveLocale(data.settings.language));
      updateCheckLocal = data.settings.update_check;
      theme = data.settings.theme === 'light' ? 'light' : 'dark';
      viewMode = ['edit', 'split', 'preview'].includes(data.settings.view_mode)
        ? data.settings.view_mode
        : 'split';
      activeVault = data.active_vault;
      vaults = data.settings.vaults;
      items = data.items;
      if (data.pair_info) p2p.setPairInfo(data.pair_info);
      if (!data.settings.has_seen_welcome) {
        isWelcomeOpen = true;
      }

      if (items.length > 0 && !selectedNotePath) {
        const firstNote = items.find((i) => !i.is_dir);
        if (firstNote) {
          await openNote(firstNote.path);
        }
      }
    } catch (e) {
      console.error('Failed to load initial state:', e);
    } finally {
      await recovery.refresh();
    }
  }

  async function refreshItems() {
    try {
      items = await listNotes();
      if (selectedNotePath && !items.some((item) => !item.is_dir && item.path === selectedNotePath)) {
        selectedNotePath = '';
        currentNoteContent = '';
        currentCrdtBase64 = '';
      }
    } catch (e) {
      console.error('Failed to refresh notes:', e);
    }
  }

  function handleOpenWikilink(token: string, kind: 'wiki' | 'markdown' = 'wiki') {
    const path = resolveNoteLink(items, selectedNotePath, token, kind);
    if (path) {
      openNote(path);
    }
  }

  // The editor reads the note itself once its listeners are ready, so the
  // page no longer reads it a second time. If that read fails, the previous
  // note is shown again.
  let previousNotePath = '';
  async function openNote(path: string) {
    if (path !== selectedNotePath) previousNotePath = selectedNotePath;
    currentNoteContent = '';
    currentCrdtBase64 = '';
    selectedNotePath = path;
    isGraphOpen = false;
  }
  function handleNoteLoadFailed(path: string) {
    if (selectedNotePath !== path) return;
    selectedNotePath = previousNotePath !== path ? previousNotePath : '';
    previousNotePath = '';
  }

  async function handleNavigateToSource(path: string, line: number) {
    if (selectedNotePath !== path) {
      await openNote(path);
    }
    targetLine = { line, nonce: Date.now() };
  }

  /** Shows a vault returned by selectVault, then `notePath` or else its first note. */
  async function showSelectedVault(res: InitialStateResponse, notePath?: string | null) {
    if (res.active_vault?.id !== activeVault?.id) {
      isHistoryOpen = false;
      isGraphOpen = false;
      selectedNotePath = '';
      currentNoteContent = '';
      currentCrdtBase64 = '';
    }
    activeVault = res.active_vault;
    lastUndoableAction = 'text';
    settings = res.settings;
    vaults = res.settings.vaults;
    items = res.items;
    if (res.pair_info) p2p.setPairInfo(res.pair_info);
    else void p2p.refreshPairInfo();

    const note = items.find((i) => !i.is_dir && i.path === notePath)
      ?? (selectedNotePath ? undefined : items.find((i) => !i.is_dir));
    if (note) await openNote(note.path);
  }

  async function handleOpenVaultFolder() {
    const path = await pickVaultDirectory();
    if (path) await showSelectedVault(await selectVault(path));
  }

  // A folder or note from "Open with LowNotes" or the command line. The native
  // side keeps the latest one until the page has loaded and takes it.
  let acceptsOpenRequests = false;
  async function handleOpenRequest() {
    if (!acceptsOpenRequests) return;
    try {
      const request = await takeOpenRequest();
      if (!request) return;
      isSettingsOpen = false;
      if (request.vault_path !== activeVault?.path) {
        if (!request.known_vault && !(await appConfirm(ts('app.openFolderAsVault', { path: request.vault_path }), { confirmLabel: ts('app.openFolderAsVaultAction') }))) return;
        await showSelectedVault(await selectVault(request.vault_path), request.note_path);
      } else if (request.note_path && items.some((i) => !i.is_dir && i.path === request.note_path)) {
        await openNote(request.note_path);
      }
    } catch (error) {
      void appAlert(trError(String(error)));
    }
  }

  function openSettings(tab: 'general' | 'themes' | 'ai' | 'providers' | 'web' | 'about' = 'general') {
    settingsTab = tab;
    isSettingsOpen = true;
  }

  async function closeSettings() {
    if (selectedNotePath) await openNote(selectedNotePath);
    isSettingsOpen = false;
  }

  function handleSettingsChange(next: AppSettings) {
    settings = next;
    theme = next.theme;
    viewMode = next.view_mode;
    updateCheckLocal = next.update_check;
    locale.set(resolveLocale(next.language));
  }

  async function handleViewModeChange(next: ViewMode) {
    const previous = viewMode;
    viewMode = next;
    try {
      await saveViewMode(next);
      if (settings) settings.view_mode = next;
    } catch (error) {
      viewMode = previous;
      console.error('Failed to save view mode:', error);
    }
  }

  // The native window starts hidden so the webview's white background never
  // shows. It appears once the themed page is rendered, with the page
  // background behind the webview. Animation frames may not run in a hidden
  // window, so this does not wait for one.
  let windowShown = false;
  function syncWindowBackground() {
    windowShown = true;
    const background = getComputedStyle(document.documentElement).getPropertyValue('--bg-main').trim();
    void windowPainted(background).catch(() => {});
  }

  $effect(() => {
    const palettes = settings?.theme_palettes;
    applyTheme(palettes?.active_palette_id ?? DEFAULT_PALETTE_ID, theme, palettes?.custom_palettes ?? []);
    if (windowShown) syncWindowBackground();
  });

  $effect(() => {
    document.documentElement.lang = $locale;
  });

  let unlistenOpenRequest: UnlistenFn | undefined;
  onMount(async () => {
    window.addEventListener('keydown', handleUndoDeletedItem, true);
    recovery.start();
    // Listeners first, so no event is lost while the initial state loads.
    await Promise.all([
      p2p.start(),
      presence.start(),
      listen('app:open-request', () => void handleOpenRequest()).then((unlisten) => (unlistenOpenRequest = unlisten)),
    ]);
    await loadInitialData();
    await tick();
    syncWindowBackground();
    acceptsOpenRequests = true;
    await handleOpenRequest();
    updates.start();
    if (!p2p.state.pairCode) await p2p.refreshPairInfo();
  });

  onDestroy(() => {
    window.removeEventListener('keydown', handleUndoDeletedItem, true);
    unlistenOpenRequest?.();
    recovery.stop();
    p2p.stop();
    presence.stop();
    updates.stop();
  });
</script>

<div class="flex h-screen w-screen overflow-hidden bg-[var(--bg-main)] text-[var(--text-main)]">
  {#if settings?.credential_error && !isSettingsOpen}
    <div class="fixed top-3 right-3 z-40 max-w-lg p-4 rounded-lg border border-[var(--danger)] bg-[var(--bg-card)] shadow-lg" role="alert">
      <p class="text-sm">{trError(settings.credential_error)}</p>
      <button class="mt-3 text-sm underline text-[var(--accent-light)]" onclick={() => openSettings()}>{$t('credentials.openSettings')}</button>
    </div>
  {/if}
  {#if recovery.state.notices.length}
    <div class="fixed top-3 right-3 z-50 max-w-lg p-4 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] shadow-lg" role="status" aria-live="polite">
      {#each recovery.state.notices as notice}
        <p class="text-sm font-medium">{notice.recovered ? $t('storage.recovered') : $t('storage.failed')}</p>
        <p class="text-xs text-[var(--text-muted)] break-all mt-1">{notice.path}</p>
      {/each}
      <button class="mt-3 text-sm underline text-[var(--accent-light)]" onclick={recovery.dismiss}>{$t('app.dismissConflict')}</button>
    </div>
  {/if}
  {#if isSettingsOpen && settings}
    <SettingsView {settings} initialTab={settingsTab} onClose={() => void closeSettings()} onChange={handleSettingsChange} />
  {:else if !activeVault}
    <!-- Welcome screen when no vault is configured -->
    <main class="flex-1 flex flex-col items-center justify-center p-8 text-center select-none">
      <button class="absolute top-5 right-6 text-sm text-[var(--text-muted)] hover:text-[var(--accent-light)]" onclick={() => openSettings()}>{$t('settings.title')}</button>
      <div class="w-16 h-16 rounded-2xl bg-[var(--bg-card)] border border-[var(--border)] flex items-center justify-center text-3xl mb-6 shadow-xl">
        <NotebookPen size={32} class="text-[var(--accent-light)]" />
      </div>
      <h1 class="text-2xl font-bold mb-2">{$t('app.welcomeTitle')}</h1>
      <p class="text-sm text-[var(--text-muted)] max-w-md mb-8 leading-relaxed">
        {$t('app.welcomeSubtitle')}
      </p>

      <button
        onclick={handleOpenVaultFolder}
        class="px-6 py-3 bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-black font-semibold text-sm rounded-xl transition shadow-lg flex items-center gap-2 hover:scale-[1.02] active:scale-[0.98]"
      >
        <FolderOpen size={18} />
        <span>{$t('app.chooseVaultFolder')}</span>
      </button>

      <p class="text-xs text-[var(--text-dim)] mt-6">
        {$t('app.filesReadable')}
      </p>
    </main>
  {:else}
    <!-- Main App Layout -->
    <Sidebar
      {activeVault}
      {vaults}
      {items}
      selectedPath={selectedNotePath}
      syncStatus={p2p.state.syncStatus}
      peerCount={activeVault.peers.length}
      onOpenSettings={() => openSettings()}
      onOpenHistory={() => (isHistoryOpen = true)}
      onSelectNote={(path) => openNote(path)}
      onVaultChange={(v) => {
        isHistoryOpen = false;
        activeVault = v;
        lastUndoableAction = 'text';
        selectedNotePath = '';
        isGraphOpen = false;
        currentNoteContent = '';
        currentCrdtBase64 = '';
        p2p.setPairInfo(null);
        refreshItems();
        void p2p.refreshPairInfo();
      }}
      onVaultsChange={(state) => {
        const switched = state.active_vault?.id !== activeVault?.id;
        settings = state.settings;
        vaults = state.settings.vaults;
        items = state.items;
        if (switched) {
          isHistoryOpen = false;
          isGraphOpen = false;
          selectedNotePath = '';
          currentNoteContent = '';
          currentCrdtBase64 = '';
          p2p.setPairInfo(state.pair_info);
        }
        activeVault = state.active_vault;
      }}
      onOpenPairModal={() => (isPairModalOpen = true)}
      onRefreshItems={refreshItems}
      onItemDeleted={() => (lastUndoableAction = 'delete')}
      presence={presence.editors}
    />

    <!-- Editor Surface -->
    <div class="flex-1 flex flex-col min-w-0 h-full overflow-hidden bg-[var(--bg-main)]">
      {#each p2p.state.conflicts as conflict (conflict.conflict_path)}
        <div role="alert" class="flex items-center gap-3 px-4 py-2 border-b border-[var(--danger)] bg-[var(--bg-card)] text-xs">
          <div class="flex-1 min-w-0">
            <strong class="text-[var(--accent-light)]">{$t('app.conflictDetected')}</strong>
            <span class="ml-2 text-[var(--text-muted)]">{$t('app.conflictExplanation')}</span>
            <span class="block truncate mt-1 text-[var(--text-dim)]">{conflict.note_path} → {conflict.conflict_path}</span>
          </div>
          <button class="shrink-0 rounded px-3 py-1.5 bg-[var(--accent)] text-[var(--accent-contrast)] font-semibold" onclick={() => { void openNote(conflict.conflict_path); p2p.dismissConflict(conflict.conflict_path); }}>{$t('app.openConflict')}</button>
          <button class="shrink-0 px-2 py-1 text-[var(--text-muted)] hover:text-[var(--text-main)]" aria-label={$t('app.dismissConflict')} title={$t('app.dismissConflict')} onclick={() => p2p.dismissConflict(conflict.conflict_path)}>×</button>
        </div>
      {/each}
      {#if isGraphOpen}
        <GraphView
          {items}
          vaultId={activeVault.id}
          onClose={() => (isGraphOpen = false)}
          onOpenNote={(path) => void openNote(path)}
        />
      {:else if selectedNotePath}
        {#key selectedNotePath}
        <Editor
          notePath={selectedNotePath}
          initialContent={currentNoteContent}
          crdtUpdateBase64={currentCrdtBase64}
          {targetLine}
          {theme}
          {viewMode}
          lineWrapping={settings?.line_wrapping ?? true}
          loadRemoteImages={settings?.load_remote_images === true}
          onLoadFailed={handleNoteLoadFailed}
          imageUploadProvider={settings?.image_upload?.provider ?? 'local'}
          vaultId={settings?.active_vault_id ?? ''}
          onViewModeChange={handleViewModeChange}
          {isAiChatOpen}
          onToggleAiChat={() => (isAiChatOpen = !isAiChatOpen)}
          onOpenNote={(path) => openNote(path)}
          onOpenGraph={() => (isGraphOpen = true)}
          onLocalEdit={() => (lastUndoableAction = 'text')}
          onOpenWikilink={handleOpenWikilink}
          deviceName={settings?.device_name ?? ''}
          deviceId={p2p.state.endpointId}
        />
        {/key}
      {:else}
        <div class="flex-1 flex flex-col min-h-0 select-none text-[var(--text-dim)]">
          <header class="app-topbar flex items-center justify-between gap-2 px-4 border-b border-[var(--border)] bg-[var(--bg-sidebar)]">
            <button
              onclick={() => (isGraphOpen = true)}
              class="px-3 py-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-hover)] inline-flex items-center gap-1.5 text-xs text-[var(--text-muted)] hover:text-[var(--accent-light)] transition"
            ><Network size={15} /> {$t('graph.button')}</button>
            <button
              onclick={() => (isAiChatOpen = !isAiChatOpen)}
              class="px-3 py-1.5 rounded-lg border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-hover)] text-xs text-[var(--text-muted)] hover:text-[var(--accent-light)] flex items-center gap-1.5 transition shadow"
            >
              <MessageSquare size={15} />
              <span>{$t('app.openAiChat')}</span>
            </button>
          </header>
          <div class="flex-1 flex flex-col items-center justify-center text-center p-8">
            <FileText size={42} class="mb-3 opacity-60" />
            <p class="text-sm">{$t('app.emptyState')}</p>
          </div>
        </div>
      {/if}
    </div>

  {/if}

  <!-- Keep chat mounted while settings are open so pending replies and drafts survive. -->
  {#if isHistoryOpen && activeVault}
    {#key activeVault.id}
      <HistoryModal vaultId={activeVault.id} vaultName={activeVault.name} initialPath={selectedNotePath} {items}
        onClose={() => (isHistoryOpen = false)}
        onUpdated={async (path) => { await refreshItems(); if (path) await openNote(path); }} />
    {/key}
  {/if}
  {#if settings && activeVault}
    <div style:display={isSettingsOpen ? 'none' : 'contents'}>
      {#key activeVault.id}
        <AiChatSidebar
          bind:isOpen={isAiChatOpen}
          bind:aiSettings={settings.ai}
          vaultId={activeVault.id}
          currentNotePath={selectedNotePath}
          onNavigateToSource={handleNavigateToSource}
          onNotesCreated={refreshItems}
          onOpenSettings={openSettings}
        />
      {/key}
    </div>
  {/if}

  <!-- Modals -->
  <PairModal
    bind:isOpen={isPairModalOpen}
    bind:pairCode={p2p.state.pairCode}
    bind:endpointId={p2p.state.endpointId}
    vaultName={activeVault?.name || ''}
    peers={activeVault?.peers || []}
    onPeersChange={async () => {
      const data = await getAppState();
      activeVault = data.active_vault;
    }}
  />

  <IncomingPairDialog
    request={p2p.state.incomingRequest}
    onAnswer={() => {
      p2p.state.incomingRequest = null;
    }}
  />

  <WelcomeModal
    bind:isOpen={isWelcomeOpen}
    onOpenAiChat={() => {
      isAiChatOpen = true;
    }}
  />

  <UpdateModal
    bind:isOpen={updates.state.open}
    update={updates.state.pending}
    policy={updates.state.policy}
    bind:autoCheck={updateCheckLocal}
    onAutoCheckChange={async (v) => {
      updateCheckLocal = v;
      await saveUpdatePrefs(v, settings?.skipped_version ?? '');
      if (settings) settings.update_check = v;
    }}
    onSkip={async (v) => {
      await saveUpdatePrefs(updateCheckLocal, v);
      if (settings) {
        settings.skipped_version = v;
      }
      updates.state.open = false;
    }}
  />
</div>

<!-- In-app alert/confirm/prompt (see $lib/dialogs.svelte.ts). -->
{#if dialogState.current}
  {#key dialogState.current}<AppDialog request={dialogState.current} />{/key}
{/if}
<ContextMenu />
