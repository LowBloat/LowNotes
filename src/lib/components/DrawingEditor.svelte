<script lang="ts">
  import { LoaderCircle, X } from 'lucide-svelte';
  import { onDestroy, onMount, untrack } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { locale, t } from '$lib/i18n';
  import type { DrawingEditorHandle } from '$lib/excalidraw';
  import type { AppTheme } from '../types';

  let { content, theme, onChange, onClose } = $props<{
    content: string;
    theme: AppTheme;
    onChange: (content: string) => void;
    onClose: () => void;
  }>();

  let host: HTMLDivElement;
  let handle: DrawingEditorHandle | null = null;
  let status = $state<'loading' | 'ready' | 'error'>('loading');
  let disposed = false;

  /** Excalidraw's own languages: Portuguese and Spanish match, English is plain `en`. */
  const LANGUAGES: Record<string, string> = { 'pt-BR': 'pt-BR', 'es-ES': 'es-ES' };

  export function merge(next: string) { handle?.merge(next); }
  export function flush() { handle?.flush(); }

  function openLink(url: string) { void openUrl(url).catch(() => {}); }

  // Links in Excalidraw's help dialog would otherwise navigate the app window.
  function handleLinkClick(event: MouseEvent) {
    const link = (event.target as Element | null)?.closest('a[href]');
    if (!link || !host.contains(link)) return;
    event.preventDefault();
    const href = link.getAttribute('href') ?? '';
    if (/^https?:\/\//i.test(href)) openLink(href);
  }

  onMount(async () => {
    try {
      const { mountDrawingEditor } = await import('$lib/excalidraw');
      const mounted = await mountDrawingEditor(host, {
        content: untrack(() => content),
        theme: untrack(() => theme),
        langCode: LANGUAGES[$locale] ?? 'en',
        onChange: (next) => onChange(next),
        onOpenLink: openLink,
      });
      if (disposed) { mounted.destroy(); return; }
      handle = mounted;
      status = 'ready';
    } catch (error) {
      console.error('Failed to open the drawing editor:', error);
      if (!disposed) status = 'error';
    }
  });

  onDestroy(() => {
    disposed = true;
    handle?.destroy();
    handle = null;
  });
</script>

<!-- Below the title bar, which keeps the window buttons. -->
<div data-modal-backdrop class="fixed inset-x-0 bottom-0 top-[var(--titlebar-height)] z-50 flex flex-col bg-[var(--bg-main)]" role="dialog" aria-modal="true" aria-labelledby="drawing-editor-title">
  <header class="app-topbar flex items-center gap-3 px-4 border-b border-[var(--border)] bg-[var(--bg-sidebar)] select-none">
    <h2 id="drawing-editor-title" class="text-sm font-semibold text-[var(--text-main)]">{$t('drawing.title')}</h2>
    <span class="text-xs text-[var(--text-dim)]">{$t('drawing.autosave')}</span>
    <button
      type="button"
      onclick={onClose}
      class="ml-auto px-2.5 py-1 text-xs rounded transition flex items-center gap-1.5 border border-[var(--border)] bg-[var(--bg-card)] hover:bg-[var(--bg-hover)] text-[var(--text-muted)] hover:text-[var(--text-main)]"
    >
      <X size={14} /> {$t('drawing.close')}
    </button>
  </header>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="relative flex-1 min-h-0" onclickcapture={handleLinkClick}>
    <div bind:this={host} class="drawing-editor-canvas absolute inset-0"></div>
    {#if status === 'loading'}
      <div role="status" class="absolute inset-0 flex items-center justify-center gap-2 text-sm text-[var(--text-muted)]">
        <LoaderCircle size={16} class="animate-spin motion-reduce:animate-none" aria-hidden="true" /> {$t('drawing.loading')}
      </div>
    {:else if status === 'error'}
      <div role="alert" class="absolute inset-0 flex items-center justify-center text-sm text-[var(--danger)]">{$t('drawing.loadError')}</div>
    {/if}
  </div>
</div>
