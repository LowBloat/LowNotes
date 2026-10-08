<script lang="ts">
  import type { VaultConfig, VaultItem } from '../types';
  import type { PresenceUser } from '$lib/presence';
  import { visibleNoteRows } from '$lib/note-tree';
  import { ChevronDown, ChevronRight, Clock3, FilePlus2, FileText, Folder, FolderOpen, FolderPlus, NotebookPen, Pencil, RefreshCw, Settings2, Trash2, X } from 'lucide-svelte';
  import {
    createNote,
    createFolder,
    deleteItem,
    renameItem,
    pickVaultDirectory,
    selectVault,
    networkSyncNow,
  } from '../api';
  import { t, trError, ts } from '$lib/i18n';

  let {
    activeVault = null,
    vaults = [],
    items = [],
    selectedPath = '',
    syncStatus = 'idle',
    peerCount = 0,
    onOpenSettings,
    onOpenHistory,
    onSelectNote,
    onVaultChange,
    onOpenPairModal,
    presence = [],
    onRefreshItems,
    onItemDeleted,
  } = $props<{
    activeVault: VaultConfig | null;
    vaults: VaultConfig[];
    items: VaultItem[];
    selectedPath: string;
    syncStatus: 'idle' | 'syncing' | 'synced' | 'error';
    peerCount: number;
    onOpenSettings: () => void;
    onOpenHistory: () => void;
    onSelectNote: (path: string) => void;
    onVaultChange: (vault: VaultConfig) => void;
    onOpenPairModal: () => void;
    onRefreshItems: () => void;
    onItemDeleted?: () => void;
    presence?: PresenceUser[];
  }>();

  let searchQuery = $state('');
  let isVaultDropdownOpen = $state(false);
  let isCreatingNote = $state(false);
  let newNoteName = $state('');
  let isCreatingFolder = $state(false);
  let newFolderName = $state('');
  let collapsedFolders = $state<Set<string>>(new Set());
  let visibleRows = $derived(visibleNoteRows(items, collapsedFolders, searchQuery));

  function toggleFolder(path: string) {
    const next = new Set(collapsedFolders);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    collapsedFolders = next;
  }

  async function handleOpenFolder() {
    isVaultDropdownOpen = false;
    const path = await pickVaultDirectory();
    if (path) {
      const res = await selectVault(path);
      if (res.active_vault) {
        onVaultChange(res.active_vault);
      }
    }
  }

  async function handleSelectVault(v: VaultConfig) {
    isVaultDropdownOpen = false;
    const res = await selectVault(v.path);
    if (res.active_vault) {
      onVaultChange(res.active_vault);
    }
  }

  async function submitNewNote() {
    if (!newNoteName.trim()) return;
    try {
      const path = await createNote(newNoteName.trim(), newNoteName.trim().split('/').at(-1) || newNoteName.trim());
      isCreatingNote = false;
      newNoteName = '';
      onRefreshItems();
      onSelectNote(path);
    } catch (e: any) {
      alert(trError(typeof e === 'string' ? e : e.message || 'sidebar.errorCreateNote'));
    }
  }

  async function submitNewFolder() {
    if (!newFolderName.trim()) return;
    try {
      await createFolder(newFolderName.trim());
      isCreatingFolder = false;
      newFolderName = '';
      onRefreshItems();
    } catch (e: any) {
      alert(trError(typeof e === 'string' ? e : e.message || 'sidebar.errorCreateFolder'));
    }
  }

  async function handleDelete(path: string, event: MouseEvent) {
    event.stopPropagation();
    if (confirm(ts('sidebar.confirmDelete', { path }))) {
      try {
        await deleteItem(path);
        onItemDeleted?.();
        onRefreshItems();
      } catch (e) {
        alert(trError(String(e)));
      }
    }
  }

  async function handleRename(oldPath: string, event: MouseEvent) {
    event.stopPropagation();
    const newName = prompt(ts('sidebar.renamePrompt'), oldPath);
    if (newName && newName !== oldPath) {
      try {
        await renameItem(oldPath, newName);
        onRefreshItems();
        if (selectedPath === oldPath) {
          onSelectNote(newName);
        } else if (selectedPath?.startsWith(`${oldPath}/`)) {
          onSelectNote(`${newName}${selectedPath.slice(oldPath.length)}`);
        }
      } catch (e: any) {
        alert(trError(typeof e === 'string' ? e : e.message || 'sidebar.errorRename'));
      }
    }
  }

</script>

<aside class="w-64 h-full flex flex-col border-r border-[var(--border)] bg-[var(--bg-sidebar)] select-none">
  <!-- Vault Switcher Header -->
  <div class="app-topbar relative flex items-center border-b border-[var(--border)] px-3">
    <button
      onclick={() => (isVaultDropdownOpen = !isVaultDropdownOpen)}
      aria-expanded={isVaultDropdownOpen}
      class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg hover:bg-[var(--bg-hover)] text-left transition group"
    >
      <div class="flex items-center gap-2 overflow-hidden">
        <NotebookPen size={16} class="shrink-0 text-[var(--accent-light)]" />
        <span class="text-xs font-semibold text-[var(--text-main)] truncate">
          {activeVault ? activeVault.name : $t('sidebar.selectVault')}
        </span>
      </div>
      <ChevronDown size={14} class="shrink-0 text-[var(--text-dim)] group-hover:text-[var(--text-muted)]" />
    </button>

    <!-- Vault Dropdown -->
    {#if isVaultDropdownOpen}
      <div class="absolute top-full left-2 right-2 mt-1 bg-[var(--bg-card)] border border-[var(--border)] rounded-lg shadow-xl py-1.5 z-40 animate-fadeIn">
        <div class="px-3 py-1 text-[10px] font-semibold text-[var(--text-dim)] uppercase tracking-wider">
          {$t('sidebar.yourVaults')}
        </div>
        {#each vaults as v}
          <button
            onclick={() => handleSelectVault(v)}
            class="w-full flex items-center justify-between px-3 py-1.5 text-xs text-left text-[var(--text-muted)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition {activeVault?.id === v.id ? 'text-[var(--accent-light)] font-medium' : ''}"
          >
            <span class="truncate">{v.name}</span>
            {#if activeVault?.id === v.id}
              <span class="text-[10px] text-[var(--accent-light)]">{$t('sidebar.active')}</span>
            {/if}
          </button>
        {/each}
        <div class="h-[1px] bg-[var(--border)] my-1"></div>
        <button
          onclick={handleOpenFolder}
          class="w-full flex items-center gap-2 px-3 py-1.5 text-xs text-left text-[var(--accent-light)] hover:bg-[var(--bg-hover)] transition font-medium"
        >
          <FolderOpen size={15} />
          <span>{$t('sidebar.openComputerFolder')}</span>
        </button>
      </div>
    {/if}
  </div>

  <!-- Search & Actions -->
  <div class="p-3 flex flex-col gap-2 border-b border-[var(--border)]">
    <div class="relative">
      <input
        type="text"
        bind:value={searchQuery}
        placeholder={$t('sidebar.searchPlaceholder')}
        class="w-full bg-[var(--bg-main)] border border-[var(--border)] rounded-md px-2.5 py-1 text-xs text-[var(--text-main)] placeholder-[var(--text-dim)] focus:outline-none focus:border-[var(--accent)]"
      />
      {#if searchQuery}
        <button
          onclick={() => (searchQuery = '')}
          title={$t('sidebar.clearSearch')}
          aria-label={$t('sidebar.clearSearch')}
          class="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-[var(--text-dim)] hover:text-[var(--text-main)]"
        >
          <X size={14} />
        </button>
      {/if}
    </div>

    <!-- Quick Buttons -->
    <div class="flex gap-1.5">
      <button
        onclick={() => (isCreatingNote = true)}
        class="flex-1 inline-flex items-center justify-center gap-1.5 py-1 px-2 text-xs font-medium rounded bg-[var(--bg-card)] hover:bg-[var(--bg-hover)] border border-[var(--border)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition text-center"
      >
        <FilePlus2 size={14} /> {$t('sidebar.newNote')}
      </button>
      <button
        onclick={() => (isCreatingFolder = true)}
        class="inline-flex items-center justify-center py-1 px-2 text-xs font-medium rounded bg-[var(--bg-card)] hover:bg-[var(--bg-hover)] border border-[var(--border)] text-[var(--text-muted)] hover:text-[var(--text-main)] transition"
        title={$t('sidebar.newFolder')}
        aria-label={$t('sidebar.newFolder')}
      >
        <FolderPlus size={16} />
      </button>
    </div>

    <!-- Inline Create Note Form -->
    {#if isCreatingNote}
      <form
        onsubmit={(e) => { e.preventDefault(); submitNewNote(); }}
        class="flex flex-col gap-1.5 p-2 bg-[var(--bg-card)] border border-[var(--accent)] rounded-md"
      >
        <input
          type="text"
          bind:value={newNoteName}
          placeholder={$t('sidebar.noteNamePlaceholder')}
          class="w-full bg-[var(--bg-main)] border border-[var(--border)] rounded px-2 py-1 text-xs text-[var(--text-main)] focus:outline-none"
        />
        <div class="flex justify-end gap-1">
          <button
            type="button"
            onclick={() => (isCreatingNote = false)}
            class="px-2 py-0.5 text-[11px] text-[var(--text-dim)] hover:text-[var(--text-main)]"
          >
            {$t('sidebar.cancel')}
          </button>
          <button
            type="submit"
            class="px-2 py-0.5 text-[11px] bg-[var(--accent)] text-black font-semibold rounded"
          >
            {$t('sidebar.create')}
          </button>
        </div>
      </form>
    {/if}

    <!-- Inline Create Folder Form -->
    {#if isCreatingFolder}
      <form
        onsubmit={(e) => { e.preventDefault(); submitNewFolder(); }}
        class="flex flex-col gap-1.5 p-2 bg-[var(--bg-card)] border border-[var(--accent)] rounded-md"
      >
        <input
          type="text"
          bind:value={newFolderName}
          placeholder={$t('sidebar.folderNamePlaceholder')}
          class="w-full bg-[var(--bg-main)] border border-[var(--border)] rounded px-2 py-1 text-xs text-[var(--text-main)] focus:outline-none"
        />
        <div class="flex justify-end gap-1">
          <button
            type="button"
            onclick={() => (isCreatingFolder = false)}
            class="px-2 py-0.5 text-[11px] text-[var(--text-dim)] hover:text-[var(--text-main)]"
          >
            {$t('sidebar.cancel')}
          </button>
          <button
            type="submit"
            class="px-2 py-0.5 text-[11px] bg-[var(--accent)] text-black font-semibold rounded"
          >
            {$t('sidebar.createFolder')}
          </button>
        </div>
      </form>
    {/if}
  </div>

  <!-- Folder tree and notes -->
  <div class="flex-1 overflow-y-auto px-2 py-2 flex flex-col gap-0.5">
    {#if visibleRows.length === 0}
      <div class="text-center py-8 text-xs text-[var(--text-dim)]">
        {searchQuery ? $t('sidebar.noNotesFound') : $t('sidebar.noNotesInVault')}
      </div>
    {:else}
      {#each visibleRows as { item, depth } (item.path)}
        <div
          role="button"
          tabindex="0"
          onclick={() => item.is_dir ? toggleFolder(item.path) : onSelectNote(item.path)}
          onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); item.is_dir ? toggleFolder(item.path) : onSelectNote(item.path); } }}
          aria-expanded={item.is_dir ? !collapsedFolders.has(item.path) : undefined}
          aria-label={`${item.title} (${item.path})`}
          class="group w-full flex items-center justify-between px-2.5 py-1.5 rounded-md text-xs text-left cursor-pointer transition {selectedPath === item.path ? 'bg-[var(--bg-active)] text-[var(--accent-light)] font-medium' : 'text-[var(--text-muted)] hover:bg-[var(--bg-hover)] hover:text-[var(--text-main)]'}"
          style:padding-left={`${10 + depth * 16}px`}
        >
          <div class="flex items-center gap-2 overflow-hidden flex-1">
            {#if item.is_dir}
              {#if collapsedFolders.has(item.path) && !searchQuery.trim()}
                <ChevronRight size={12} class="shrink-0 text-[var(--text-dim)]" />
                <Folder size={14} class="shrink-0 text-[var(--text-dim)]" />
              {:else}
                <ChevronDown size={12} class="shrink-0 text-[var(--text-dim)]" />
                <FolderOpen size={14} class="shrink-0 text-[var(--text-dim)]" />
              {/if}
            {:else}
              <FileText size={14} class="shrink-0 text-[var(--text-dim)]" />
            {/if}
            <span class="truncate" title={item.path}>{item.is_dir ? item.name : item.title}</span>
          </div>

          <!-- Hover actions -->
          <div class="hidden group-hover:flex group-focus-within:flex items-center gap-1 opacity-80">
            {#if item.is_dir}
              <button
                onclick={(e) => { e.stopPropagation(); newNoteName = `${item.path}/`; isCreatingNote = true; }}
                class="w-4 h-4 flex items-center justify-center text-[10px] text-[var(--text-dim)] hover:text-[var(--text-main)]"
                title={$t('sidebar.newNote')}
                aria-label={$t('sidebar.newNote')}
              ><FilePlus2 size={12} /></button>
            {/if}
            <button
              onclick={(e) => handleRename(item.path, e)}
              class="w-4 h-4 flex items-center justify-center text-[10px] text-[var(--text-dim)] hover:text-[var(--text-main)]"
              title={$t('sidebar.rename')}
              aria-label={$t('sidebar.rename')}
            >
              <Pencil size={12} />
            </button>
            <button
              onclick={(e) => handleDelete(item.path, e)}
              class="w-4 h-4 flex items-center justify-center text-[10px] text-[var(--text-dim)] hover:text-red-400"
              title={$t('sidebar.delete')}
              aria-label={$t('sidebar.delete')}
            >
              <Trash2 size={12} />
            </button>
          </div>
        </div>
      {/each}
    {/if}
  </div>

  <!-- Presence: who is editing right now -->
  {#if presence.length > 0}
    <div class="px-3 py-1.5 border-t border-[var(--border)] bg-[var(--bg-card)] flex flex-col gap-1">
      {#each presence.slice(0, 3) as person (person.deviceId)}
        <span class="flex items-center gap-1.5 text-[10px] text-[var(--text-dim)]">
          <span class="w-2 h-2 rounded-full shrink-0" style="background: {person.color}"></span>
          <span class="truncate">
            {$t('presence.editingNote', { name: person.name, note: person.notePath })}
          </span>
        </span>
      {/each}
    </div>
  {/if}

  <div class="px-3 py-2 border-t border-[var(--border)]">
    <button onclick={onOpenHistory}
      class="w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg text-xs font-semibold text-[var(--text-muted)] hover:text-[var(--accent-light)] hover:bg-[var(--bg-hover)] transition"
      title={$t('history.title')}><Clock3 size={17} /> {$t('history.title')}</button>
    <button onclick={onOpenSettings}
      class="w-full flex items-center gap-2.5 px-2.5 py-2 rounded-lg text-xs font-semibold text-[var(--text-muted)] hover:text-[var(--accent-light)] hover:bg-[var(--bg-hover)] transition"
      title={$t('settings.title')}><Settings2 size={17} /> {$t('settings.title')}</button>
  </div>

  <!-- P2P Status Footer -->
  <div class="p-3 border-t border-[var(--border)] bg-[var(--bg-card)] flex items-center justify-between">
    <button
      onclick={onOpenPairModal}
      class="flex items-center gap-2 text-xs text-left hover:opacity-90 transition group"
      title={syncStatus === 'error' ? $t('sidebar.syncError') : $t('sidebar.manageConnections')}
    >
      <span
        class="inline-block w-2.5 h-2.5 rounded-full {syncStatus === 'error' ? 'bg-red-500' : syncStatus === 'syncing' ? 'bg-[var(--accent)] animate-pulse' : peerCount > 0 ? 'bg-[var(--success)]' : 'bg-gray-500'}"
      ></span>
      <div class="flex flex-col">
        <span class="font-medium text-[var(--text-main)] leading-none text-[11px]">
          {peerCount > 0
            ? peerCount === 1
              ? $t('sidebar.pairedOne')
              : $t('sidebar.pairedMany', { count: peerCount })
            : $t('sidebar.p2pOffline')}
        </span>
        <span class="text-[10px] text-[var(--text-dim)] group-hover:text-[var(--accent-light)]">
          {$t('sidebar.manageConnections')}
        </span>
      </div>
    </button>

    <div class="flex items-center gap-1">
      <button
        onclick={() => networkSyncNow().catch(console.error)}
        class="w-7 h-7 flex items-center justify-center rounded hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--accent-light)] transition"
        title={$t('sidebar.syncNow')}
        aria-label={$t('sidebar.syncNow')}
      >
        <RefreshCw size={16} />
      </button>
    </div>
  </div>
</aside>
