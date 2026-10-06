<script lang="ts">
  import { ArrowDown, ArrowUp, CaseSensitive, ChevronRight, Regex, Replace, ReplaceAll, Search, WholeWord, X } from 'lucide-svelte';
  import { t } from '$lib/i18n';
  import type { ViewMode } from '$lib/types';

  let {
    query = $bindable(''), replacement = $bindable(''), caseSensitive = $bindable(false),
    regexp = $bindable(false), wholeWord = $bindable(false), replacing = $bindable(false),
    mode, count, current, invalid, canReplace, canReplaceAll, onNavigate, onClose, onReplace,
  } = $props<{
    query: string; replacement: string; caseSensitive: boolean; regexp: boolean; wholeWord: boolean; replacing: boolean;
    mode: ViewMode; count: number; current: number; invalid: boolean; canReplace: boolean; canReplaceAll: boolean;
    onNavigate: (direction: number) => void; onClose: () => void; onReplace: (all: boolean) => void;
  }>();
  let input: HTMLInputElement;
  export function focus() { input?.focus(); input?.select(); }
</script>

<section class="note-find-panel" aria-label={$t('editor.findTitle')}>
  <div class="find-main-row">
    {#if mode !== 'preview'}
      <button class="find-icon find-disclosure" class:expanded={replacing} aria-label={$t('editor.findShowReplace')} title={$t('editor.findShowReplace')} aria-expanded={replacing} onclick={() => { replacing = !replacing; }}><ChevronRight size={15} /></button>
    {:else}
      <Search size={16} class="find-search-icon" aria-hidden="true" />
    {/if}
    <input bind:this={input} bind:value={query} class="find-input" aria-label={$t('editor.findPlaceholder')} placeholder={$t('editor.findPlaceholder')} aria-invalid={invalid} autocomplete="off" spellcheck="false"
      onkeydown={(event) => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); onNavigate(event.shiftKey ? -1 : 1); } }} />
    <span class="find-count" class:empty={query && !count} aria-live="polite" aria-atomic="true">{query && !invalid ? `${count ? current + 1 : 0} / ${count}` : '—'}</span>
    <div class="find-navigation">
      <button class="find-icon" disabled={!count} aria-label={$t('editor.findPrevious')} title={$t('editor.findPrevious')} onclick={() => onNavigate(-1)}><ArrowUp size={16} /></button>
      <button class="find-icon" disabled={!count} aria-label={$t('editor.findNext')} title={$t('editor.findNext')} onclick={() => onNavigate(1)}><ArrowDown size={16} /></button>
      <button class="find-icon" aria-label={$t('editor.findClose')} title={$t('editor.findClose')} onclick={onClose}><X size={16} /></button>
    </div>
  </div>
  <div class="find-options-row">
    <span class="find-scope">{$t(mode === 'edit' ? 'editor.findScopeEditor' : mode === 'split' ? 'editor.findScopeBoth' : 'editor.findScopePreview')}</span>
    <div class="find-options">
      <button class="find-icon" class:active={caseSensitive} aria-pressed={caseSensitive} aria-label={$t('editor.findMatchCase')} title={$t('editor.findMatchCase')} onclick={() => { caseSensitive = !caseSensitive; }}><CaseSensitive size={17} /></button>
      <button class="find-icon" class:active={wholeWord} aria-pressed={wholeWord} aria-label={$t('editor.findWholeWord')} title={$t('editor.findWholeWord')} onclick={() => { wholeWord = !wholeWord; }}><WholeWord size={17} /></button>
      <button class="find-icon" class:active={regexp} aria-pressed={regexp} aria-label={$t('editor.findRegex')} title={$t('editor.findRegex')} onclick={() => { regexp = !regexp; }}><Regex size={17} /></button>
    </div>
  </div>
  {#if invalid || (query && !count)}
    <p class="find-message" role="status">{$t(invalid ? 'editor.findInvalidRegex' : 'editor.findNoResults')}</p>
  {/if}
  {#if replacing && mode !== 'preview'}
    <div class="find-replace-row">
      <Replace size={16} class="find-search-icon" aria-hidden="true" />
      <input class="find-input" bind:value={replacement} placeholder={$t('editor.findReplacePlaceholder')} aria-label={$t('editor.findReplacePlaceholder')} autocomplete="off" spellcheck="false"
        onkeydown={(event) => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); onReplace(event.ctrlKey || event.metaKey); } }} />
      <button class="find-icon" disabled={!canReplace || invalid} aria-label={$t('editor.findReplaceOne')} title={$t('editor.findReplaceOne')} onclick={() => onReplace(false)}><Replace size={17} /></button>
      <button class="find-icon" disabled={!canReplaceAll || invalid} aria-label={$t('editor.findReplaceAll')} title={$t('editor.findReplaceAll')} onclick={() => onReplace(true)}><ReplaceAll size={17} /></button>
    </div>
  {/if}
</section>

<style>
  .note-find-panel { position: absolute; top: 12px; right: 16px; z-index: 30; width: 420px; max-width: calc(100% - 24px); padding: 6px; border: 1px solid var(--border); border-radius: 12px; background: var(--bg-card); color: var(--text-main); box-shadow: 0 8px 28px rgb(0 0 0 / .16), 0 2px 5px rgb(0 0 0 / .06); font-size: 13px; }
  .note-find-panel { container-type: inline-size; }
  .find-main-row, .find-replace-row { display: flex; align-items: center; gap: 3px; min-width: 0; }
  .find-main-row { padding: 2px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-main); }
  .find-main-row:focus-within { border-color: var(--accent-light); box-shadow: 0 0 0 1px var(--accent-glow); }
  .find-input { min-width: 0; flex: 1; width: 100%; height: 34px; padding: 0 4px; background: transparent; color: var(--text-main); border: 0; outline: 0; font-size: 13px; user-select: text; }
  .find-input::placeholder { color: var(--text-dim); }
  .find-replace-row:focus-within { border-color: var(--accent-light); }
  .find-icon { display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 30px; flex: none; border-radius: 5px; color: var(--text-muted); }
  .find-icon:hover:not(:disabled) { color: var(--text-main); background: var(--bg-hover); }
  .find-icon.active { color: var(--accent-light); background: var(--bg-active); box-shadow: inset 0 0 0 1px var(--border); }
  .find-icon.active:hover { color: var(--accent-light); }
  .find-disclosure.expanded :global(svg) { transform: rotate(90deg); }
  :global(.note-find-panel .find-search-icon) { margin: 0 6px; flex: none; color: var(--text-dim); }
  .find-count { min-width: 46px; padding: 0 5px; text-align: right; white-space: nowrap; color: var(--text-dim); font-size: 12px; font-variant-numeric: tabular-nums; }
  .find-count.empty { color: var(--danger); }
  .find-navigation { display: flex; gap: 1px; border-left: 1px solid var(--border); padding-left: 4px; margin-left: 3px; }
  .find-options-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 5px 3px 0 8px; }
  .find-scope { min-width: 0; color: var(--text-dim); font-size: 12px; }
  .find-options { display: flex; gap: 2px; }
  .find-message { margin: 4px 8px 5px; color: var(--danger); font-size: 12px; }
  .find-replace-row { margin-top: 6px; padding: 2px 5px; border: 1px solid var(--border); border-radius: 7px; background: var(--bg-main); }
  @container (max-width: 340px) {
    .find-main-row { flex-wrap: wrap; }
    .find-main-row .find-input { flex-basis: calc(100% - 36px); }
    .find-count { flex: 1; text-align: left; padding-left: 6px; }
    .find-navigation { border-left: 0; }
  }
  @media (max-width: 600px) { .note-find-panel { top: 8px; right: 8px; max-width: calc(100% - 16px); } .find-icon { width: 28px; } }
</style>
