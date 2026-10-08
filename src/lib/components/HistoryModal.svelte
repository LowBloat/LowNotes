<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { ArchiveRestore, Clock3, Folder, RefreshCw, Trash2, X } from 'lucide-svelte';
  import { historyList, historyVersion, historyApply, historyTrashRead, historyTrashRestore, historyRetentionSave, historyCleanup, readNote } from '$lib/api';
  import type { VaultItem, HistoryListing, NoteVersion, TrashEntry, CleanupReport } from '$lib/types';
  import { t, ts, locale, trError } from '$lib/i18n';
  import { dismissibleModal } from '$lib/modal-dismiss';

  let { vaultId, vaultName, initialPath = '', items, onClose, onUpdated } = $props<{
    vaultId: string; vaultName: string; initialPath?: string; items: VaultItem[];
    onClose: () => void; onUpdated: (path?: string) => Promise<void>;
  }>();
  let tab = $state<'versions' | 'trash' | 'retention'>('versions');
  let path = $state('');
  let data = $state<HistoryListing | null>(null);
  let busy = $state(false), error = $state(''), notice = $state('');
  let selected = $state<NoteVersion | null>(null), deleted = $state<TrashEntry | null>(null);
  let other = $state(''), otherLabel = $state(''), merge = $state('');
  let hasOther = $state(false), comparePath = $state('');
  let versionsDays = $state(''), trashDays = $state('');
  let preview = $state<CleanupReport | null>(null);
  let panel: HTMLDivElement | undefined = $state();
  const notes = $derived<VaultItem[]>(items.filter((item: VaultItem) => !item.is_dir));
  const retentionChanged = $derived(versionsDays !== (data?.retention.versions_days?.toString() ?? '')
    || trashDays !== (data?.retention.trash_days?.toString() ?? ''));
  let alive = true;

  async function run(action: () => Promise<void>) {
    if (busy) return;
    busy = true; error = ''; notice = '';
    try { await action(); } catch (cause) { if (alive) error = trError(String(cause)); }
    finally { if (alive) busy = false; }
  }
  async function reload(next = path, keepResult = false) {
    const result = await historyList(vaultId, next);
    if (!alive) return;
    const keep = keepResult && !!data?.note && data.note.note_id === result.note?.note_id;
    data = result; path = result.note?.path ?? '';
    if (!keep) { merge = result.note?.content ?? ''; selected = null; other = ''; otherLabel = ''; hasOther = false; comparePath = ''; }
    deleted = null;
    versionsDays = result.retention.versions_days?.toString() ?? '';
    trashDays = result.retention.trash_days?.toString() ?? '';
    preview = null;
  }
  function date(milliseconds: number) { return new Date(milliseconds).toLocaleString($locale); }
  async function chooseVersion(version: NoteVersion) {
    await run(async () => {
      const result = await historyVersion(vaultId, version.note_id, version.id);
      if (!alive) return;
      selected = version; other = result.content; otherLabel = date(version.created_ms); hasOther = true; comparePath = '';
      merge = data?.note?.content ?? '';
    });
  }
  async function chooseNote(value: string) {
    await run(async () => {
      if (!value) { hasOther = false; other = ''; selected = null; return; }
      const result = await readNote(value);
      if (!alive) return;
      other = result.content; otherLabel = value; hasOther = true; selected = null;
      merge = data?.note?.content ?? '';
    });
  }
  async function apply(useVersion: boolean) {
    await run(async () => {
      const note = data?.note;
      if (!note) return;
      const updated = await historyApply(vaultId, note.note_id, note.hash,
        useVersion && selected ? { versionId: selected.id } : { content: merge });
      if (!alive) return;
      await onUpdated(updated); await reload(updated); notice = ts('history.applied');
    });
  }
  async function chooseTrash(entry: TrashEntry) {
    await run(async () => {
      const content = entry.is_dir ? '' : await historyTrashRead(vaultId, entry);
      if (!alive) return;
      deleted = entry; other = content;
    });
  }
  async function restoreTrash() {
    await run(async () => {
      if (!deleted) return;
      const result = await historyTrashRestore(vaultId, deleted);
      if (!alive) return;
      await onUpdated(result.is_dir ? undefined : result.path); await reload();
      notice = ts('history.restored', { path: result.path });
    });
  }
  function policyDays(value: string): number | null {
    if (!value.trim()) return null;
    const days = Number(value);
    if (!Number.isInteger(days) || days < 1 || days > 36500) throw 'history.invalidRetention';
    return days;
  }
  async function savePolicy() {
    await run(async () => {
      const policy = { versions_days: policyDays(versionsDays), trash_days: policyDays(trashDays) };
      await historyRetentionSave(vaultId, policy);
      if (!alive) return;
      if (data) data = { ...data, retention: policy };
      versionsDays = policy.versions_days?.toString() ?? ''; trashDays = policy.trash_days?.toString() ?? '';
      preview = null; notice = ts('history.policySaved');
    });
  }
  async function cleanup(apply: boolean) {
    await run(async () => {
      const result = await historyCleanup(vaultId, apply);
      if (!alive) return;
      preview = result;
      if (apply) { await reload(); preview = result; notice = ts('history.cleaned', { versions: result.versions, archives: result.archives }); }
    });
  }
  function trap(event: KeyboardEvent) {
    if (event.key !== 'Tab' || !panel) return;
    const elements = [...panel.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled)')]
      .filter(element => element.getClientRects().length);
    const first = elements[0], last = elements.at(-1);
    if (event.shiftKey && (document.activeElement === first || document.activeElement === panel)) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || document.activeElement === panel)) { event.preventDefault(); first?.focus(); }
  }
  onMount(() => {
    const before = document.activeElement;
    panel?.focus();
    void run(() => reload(initialPath));
    return () => { if (before instanceof HTMLElement && before.isConnected) before.focus(); };
  });
  onDestroy(() => { alive = false; });
</script>

<div class="history-backdrop" use:dismissibleModal={() => { if (!busy) onClose(); }}>
  <div class="history-dialog" role="dialog" aria-modal="true" aria-labelledby="history-title" aria-busy={busy} tabindex="-1" bind:this={panel} onkeydown={trap}>
    <header>
      <div><h2 id="history-title"><Clock3 size={19} /> {$t('history.title')}</h2><p>{vaultName} · {$t('history.localOnly')}</p></div>
      <button class="icon-button" onclick={onClose} disabled={busy} aria-label={$t('history.close')}><X size={19} /></button>
    </header>
    <nav aria-label={$t('history.title')}>
      <button class:active={tab === 'versions'} disabled={busy} onclick={() => { tab = 'versions'; other = ''; hasOther = false; selected = null; }}><Clock3 size={15} /> {$t('history.versions')}</button>
      <button class:active={tab === 'trash'} disabled={busy} onclick={() => { tab = 'trash'; deleted = null; other = ''; }}><Trash2 size={15} /> {$t('history.trash')}</button>
      <button class:active={tab === 'retention'} disabled={busy} onclick={() => (tab = 'retention')}>{$t('history.retention')}</button>
      <button class="icon-button reload" onclick={() => run(() => reload(path, true))} disabled={busy} aria-label={$t('history.refresh')}><RefreshCw size={15} class={busy ? 'spinning' : ''} /></button>
    </nav>
    {#if error}<p class="message error" role="alert">{error}</p>{/if}
    {#if notice}<p class="message" role="status">{notice}</p>{/if}
    {#if busy}<p class="working" role="status">{$t('history.working')}</p>{/if}

    {#if tab === 'versions'}
      <div class="note-picker"><label for="history-note">{$t('history.note')}</label>
        <select id="history-note" value={path} disabled={busy} onchange={event => run(() => reload(event.currentTarget.value))}>
          <option value="">{$t('history.selectNote')}</option>{#each notes as note}<option value={note.path}>{note.path}</option>{/each}
        </select>
      </div>
      <div class="history-body">
        <aside aria-label={$t('history.versions')}>
          <p class="aside-label">{$t('history.checkpoints')}</p>
          {#each data?.versions ?? [] as version}
            <button class="entry" class:chosen={selected?.id === version.id} disabled={busy} onclick={() => chooseVersion(version)}>
              <strong>{date(version.created_ms)}</strong><span>{version.path}</span><small>{$t('history.characters', { count: version.characters })}</small>
            </button>
          {:else}<p class="empty">{$t('history.noVersions')}</p>{/each}
          <p class="aside-help">{$t('history.checkpointHelp')}</p>
        </aside>
        <section class="comparison">
          {#if data?.note}
            <label class="compare-select" for="history-other">{$t('history.compareNote')}
              <select id="history-other" bind:value={comparePath} disabled={busy} onchange={event => chooseNote(event.currentTarget.value)}>
                <option value="">{$t('history.selectComparison')}</option>{#each notes.filter((note: VaultItem) => note.path !== path) as note}<option value={note.path}>{note.path}</option>{/each}
              </select>
            </label>
            <div class="compare-panes">
              <label>{$t('history.current')}<textarea readonly value={data.note.content} spellcheck="false" aria-label={$t('history.current')}></textarea></label>
              <label>{hasOther ? otherLabel : $t('history.previous')}<textarea readonly value={other} spellcheck="false" aria-label={$t('history.previous')}></textarea></label>
            </div>
            <div class="merge-actions">
              <button disabled={busy} onclick={() => (merge = data?.note?.content ?? '')}>{$t('history.useCurrent')}</button>
              <button disabled={busy || !hasOther} onclick={() => (merge = other)}>{$t('history.usePrevious')}</button>
              <button disabled={busy || !hasOther} onclick={() => (merge = `${data?.note?.content ?? ''}\n\n${other}`)}>{$t('history.combine')}</button>
              {#if selected}<button disabled={busy} onclick={() => apply(true)}><ArchiveRestore size={14} /> {$t('history.restoreVersion')}</button>{/if}
            </div>
            <label class="merge-editor">{$t('history.result')}<textarea bind:value={merge} spellcheck="false" disabled={busy} aria-label={$t('history.result')}></textarea></label>
            <footer><p>{$t('history.applyHelp')}</p><button class="primary" disabled={busy || merge === data.note.content} onclick={() => apply(false)}>{$t('history.apply')}</button></footer>
          {:else}<p class="empty">{$t('history.selectNote')}</p>{/if}
        </section>
      </div>
    {:else if tab === 'trash'}
      <div class="history-body">
        <aside aria-label={$t('history.trash')}>
          {#each data?.trash ?? [] as entry}
            <button class="entry" class:chosen={deleted?.note_id === entry.note_id} disabled={busy} onclick={() => chooseTrash(entry)}>
              <strong>{#if entry.is_dir}<Folder size={14} />{/if}{entry.path}</strong><span>{date(entry.deleted_ms)}</span><small>{$t('history.items', { count: entry.items })}</small>
            </button>
          {:else}<p class="empty">{$t('history.emptyTrash')}</p>{/each}
        </aside>
        <section class="comparison trash-detail">
          {#if deleted}
            <h3>{deleted.path}</h3><p class="hint">{$t('history.trashHelp')}</p>
            {#if deleted.is_dir}<p class="empty">{$t('history.restoreFolderHelp', { count: deleted.items })}</p>
            {:else}<textarea class="trash-text" readonly value={other} spellcheck="false" aria-label={$t('history.deletedContent')}></textarea>{/if}
            <footer><button class="primary" disabled={busy} onclick={restoreTrash}><ArchiveRestore size={15} /> {$t('history.restore')}</button></footer>
          {:else}<p class="empty">{$t('history.selectTrash')}</p>{/if}
        </section>
      </div>
    {:else}
      <section class="retention">
        <h3>{$t('history.retention')}</h3><p class="hint">{$t('history.retentionHelp')}</p>
        <div class="retention-fields">
          <label>{$t('history.versionsDays')}<input type="number" min="1" max="36500" step="1" value={versionsDays} oninput={event => { versionsDays = event.currentTarget.value; preview = null; }} placeholder={$t('history.forever')} disabled={busy} /></label>
          <label>{$t('history.trashDays')}<input type="number" min="1" max="36500" step="1" value={trashDays} oninput={event => { trashDays = event.currentTarget.value; preview = null; }} placeholder={$t('history.forever')} disabled={busy} /></label>
        </div>
        <button class="primary" disabled={busy || !retentionChanged} onclick={savePolicy}>{$t('history.saveRetention')}</button>
        <div class="cleanup"><h3>{$t('history.cleanup')}</h3><p class="hint">{$t('history.cleanupHelp')}</p>
          <button disabled={busy || retentionChanged} onclick={() => cleanup(false)}>{$t('history.previewCleanup')}</button>
          {#if preview}<p role="status">{$t('history.cleanupPreview', { versions: preview.versions, archives: preview.archives, protected: preview.protected })}</p>
            <button class="danger" disabled={busy || retentionChanged || !(preview.versions + preview.archives)} onclick={() => cleanup(true)}>{$t('history.removeExpired')}</button>{/if}
        </div>
      </section>
    {/if}
  </div>
</div>

<style>
  .history-backdrop { position: fixed; inset: 0; z-index: 70; background: #0008; display: grid; place-items: center; padding: 16px; }
  .history-dialog { width: min(1120px, 100%); height: min(790px, calc(100dvh - 32px)); display: flex; flex-direction: column; overflow: hidden; color: var(--text-main); background: var(--bg-main); border: 1px solid var(--border); border-radius: 12px; box-shadow: 0 20px 70px #0004; }
  header { display: flex; justify-content: space-between; align-items: center; padding: 18px 22px; background: var(--bg-sidebar); border-bottom: 1px solid var(--border); }
  h2 { display: flex; align-items: center; gap: 9px; font-size: 16px; font-weight: 650; margin: 0; } h3 { font-size: 15px; font-weight: 650; }
  header p { font-size: 11px; color: var(--text-muted); margin-top: 5px; }
  nav { display: flex; gap: 5px; padding: 8px 16px; border-bottom: 1px solid var(--border); }
  button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-card); font-size: 12px; cursor: pointer; }
  button:hover:not(:disabled) { background: var(--bg-hover); } button:disabled { opacity: .5; cursor: default; }
  button:focus-visible, textarea:focus-visible, select:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  nav button { border-color: transparent; background: transparent; } nav .active, .chosen { background: var(--bg-hover); border-color: var(--accent); }
  .icon-button { width: 32px; height: 32px; padding: 0; } .reload { margin-left: auto; }
  .primary { background: var(--accent); color: var(--accent-contrast); font-weight: 650; } .primary:hover:not(:disabled) { background: var(--accent-light); }
  .message { padding: 9px 20px; font-size: 12px; background: var(--bg-card); border-bottom: 1px solid var(--border); } .error, .danger { color: var(--danger); }
  .working { padding: 4px 20px; font-size: 11px; color: var(--text-muted); }
  .note-picker { display: flex; align-items: center; gap: 10px; padding: 10px 18px; font-size: 12px; } .note-picker select { flex: 1; min-width: 0; }
  select, input { background: var(--bg-card); border: 1px solid var(--border); border-radius: 5px; color: var(--text-main); padding: 7px 9px; font-size: 12px; }
  .history-body { display: grid; grid-template-columns: 250px minmax(0, 1fr); flex: 1; min-height: 0; border-top: 1px solid var(--border); }
  aside { overflow: auto; background: var(--bg-sidebar); border-right: 1px solid var(--border); padding: 10px; }
  .entry { display: flex; align-items: stretch; flex-direction: column; width: 100%; text-align: left; margin-bottom: 7px; padding: 11px; overflow-wrap: anywhere; }
  .entry strong { display: flex; align-items: center; gap: 5px; font-size: 12px; font-weight: 600; } .entry span, .entry small { font-size: 10px; color: var(--text-muted); margin-top: 4px; }
  .aside-label { font-size: 11px; color: var(--text-muted); margin: 4px 5px 10px; } .aside-help { font-size: 10px; color: var(--text-dim); padding: 8px; line-height: 1.6; }
  .empty { color: var(--text-muted); padding: 24px 15px; font-size: 12px; line-height: 1.6; }
  .comparison { display: flex; flex-direction: column; gap: 10px; min-height: 0; padding: 16px; overflow: auto; }
  .compare-select { display: flex; align-items: center; gap: 10px; font-size: 11px; } .compare-select select { flex: 1; min-width: 0; }
  .compare-panes { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; flex: 1; min-height: 130px; }
  .compare-panes label, .merge-editor { display: flex; flex-direction: column; gap: 7px; font-size: 11px; color: var(--text-muted); min-height: 0; overflow: hidden; }
  textarea { flex: 1; min-height: 0; width: 100%; resize: none; background: var(--bg-card); border: 1px solid var(--border); border-radius: 6px; padding: 11px; color: var(--text-main); font: 12px/1.7 Consolas, 'Cascadia Code', monospace; tab-size: 4; }
  .merge-actions { display: flex; flex-wrap: wrap; gap: 5px; } .merge-actions button { font-size: 11px; }
  .merge-editor { flex: 1; min-height: 120px; } footer { display: flex; justify-content: flex-end; align-items: center; gap: 12px; } footer p { font-size: 10px; color: var(--text-muted); flex: 1; }
  .trash-text { min-height: 130px; } .hint { font-size: 12px; color: var(--text-muted); line-height: 1.7; }
  .retention { padding: 24px; overflow: auto; } .retention-fields { display: flex; gap: 20px; margin: 22px 0; } .retention-fields label { display: flex; flex-direction: column; gap: 8px; font-size: 12px; } .retention-fields input { width: 180px; }
  .cleanup { margin-top: 28px; padding-top: 24px; border-top: 1px solid var(--border); } .cleanup p { margin: 12px 0; font-size: 12px; }
  .reload :global(.spinning) { animation: spin 1s linear infinite; } @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .reload :global(.spinning) { animation: none; } }
  @media (max-width: 760px) { .history-body { grid-template-columns: 180px minmax(0, 1fr); } .comparison { padding: 10px; } .compare-select { flex-direction: column; align-items: stretch; } .compare-panes { grid-template-columns: 1fr; min-height: 180px; } .retention-fields { flex-wrap: wrap; } }
</style>
