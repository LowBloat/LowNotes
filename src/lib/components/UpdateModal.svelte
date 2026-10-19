<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { relaunch } from '@tauri-apps/plugin-process';
  import type { DownloadEvent } from '@tauri-apps/plugin-updater';
  import type { UpdatePolicy } from '$lib/types';
  import { canInstallUpdate, updateInstructionKey, type AvailableUpdate } from '$lib/updates';
  import { t } from '$lib/i18n';
  import { dismissibleModal } from '$lib/modal-dismiss';
  import { isRemoteImage, renderReleaseNotes } from '$lib/markdown';
  import { previewLinkAction } from '$lib/preview-links';

  let {
    isOpen = $bindable(false),
    update = null,
    policy = null,
    autoCheck = $bindable(true),
    onSkip,
    onAutoCheckChange,
  } = $props<{
    isOpen: boolean;
    update: AvailableUpdate | null;
    policy?: UpdatePolicy | null;
    autoCheck?: boolean;
    onSkip?: (version: string) => void;
    onAutoCheckChange?: (value: boolean) => void;
  }>();

  let phase = $state<'idle' | 'downloading' | 'installing' | 'error'>('idle');
  let percent = $state(0);
  let errorMessage = $state('');
  const canInstall = $derived(canInstallUpdate(policy, update));

  const releaseUrl = $derived(
    update ? `https://github.com/LowBloat/LowNotes/releases/tag/v${update.version}` : ''
  );

  const notesHtml = $derived(update?.body?.trim() ? renderReleaseNotes(update.body) : '');

  // Links must never navigate the webview: web links open in the browser and
  // anything else is ignored. Blocked remote images load on click.
  function handleNotesClick(event: MouseEvent) {
    const target = event.target as Element | null;
    const blocked = target?.closest<HTMLElement>('[data-remote-src]');
    if (blocked) {
      const src = blocked.dataset.remoteSrc ?? '';
      if (!isRemoteImage(src)) return;
      const image = document.createElement('img');
      image.src = src;
      image.alt = blocked.title;
      blocked.replaceWith(image);
      return;
    }
    const link = target?.closest('a');
    if (!link) return;
    event.preventDefault();
    const action = previewLinkAction(link.getAttribute('href'), null);
    if (action.kind === 'external') {
      openUrl(action.url).catch((e) => console.error('Failed to open release notes link:', e));
    }
  }

  async function handleOpenRelease() {
    if (!releaseUrl) return;
    try {
      await openUrl(releaseUrl);
    } catch (e) {
      console.error('Failed to open release URL:', e);
    }
  }

  async function handleDownloadPackage() {
    if (!update?.download_url) return;
    try {
      await openUrl(update.download_url);
    } catch (e) {
      console.error('Failed to open package download:', e);
    }
  }

  function handleSkip() {
    if (update && onSkip) {
      onSkip(update.version);
    }
    isOpen = false;
  }

  function handleAutoCheckChange(event: Event) {
    const checked = (event.target as HTMLInputElement).checked;
    autoCheck = checked;
    if (onAutoCheckChange) {
      onAutoCheckChange(checked);
    }
  }

  async function handleInstallNow() {
    if (!canInstall || !update?.installer || phase === 'downloading' || phase === 'installing') return;
    const installer = update.installer;
    phase = 'downloading';
    percent = 0;
    errorMessage = '';
    let received = 0;
    const total = installer.contentLength ?? 0;
    try {
      await installer.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === 'Progress') {
          received += event.data.chunkLength;
          if (total > 0) {
            percent = Math.min(99, Math.round((received / total) * 100));
          }
        } else if (event.event === 'Finished') {
          phase = 'installing';
        }
      });
      phase = 'installing';
      await relaunch();
    } catch (e) {
      console.error('Failed to install update:', e);
      phase = 'error';
      errorMessage = typeof e === 'string' ? e : (e as Error)?.message || String(e);
    }
  }
</script>

{#if isOpen && update}
  <div
    use:dismissibleModal={() => { if (phase !== 'downloading' && phase !== 'installing') isOpen = false; }}
    class="fixed inset-0 bg-black/70 backdrop-blur-sm z-50 flex items-center justify-center p-4 select-none animate-fadeIn"
    role="presentation"
  >
    <div
      class="bg-[var(--bg-card)] border border-[var(--border)] rounded-2xl w-full max-w-md shadow-2xl overflow-hidden flex flex-col"
      role="dialog"
      aria-modal="true"
      aria-labelledby="update-dialog-title"
      tabindex="-1"
    >
      <div class="px-6 pt-6 pb-4 border-b border-[var(--border)]">
        <h2 id="update-dialog-title" class="text-base font-bold text-[var(--text-main)] mb-1">{$t('update.title')}</h2>
        <p class="text-xs text-[var(--text-muted)]">{$t('update.body', { version: update.version })}</p>
      </div>

      <div class="px-6 py-4 max-h-64 overflow-y-auto bg-[var(--bg-main)]">
        <!-- Rendered with raw HTML disabled; clicks are handled in handleNotesClick. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div role="presentation" class="release-notes text-[11px] text-[var(--text-muted)] leading-relaxed select-text" data-testid="release-notes" onclick={handleNotesClick}>{@html notesHtml}</div>
      </div>

      {#if !canInstall}
        <div class="px-6 py-4 border-t border-[var(--border)]">
          <p class="text-xs text-[var(--text-main)] leading-relaxed">{$t(updateInstructionKey(policy))}</p>
        </div>
      {/if}

      {#if phase === 'downloading' || phase === 'installing' || phase === 'error'}
        <div class="px-6 py-3 border-t border-[var(--border)] flex flex-col gap-2">
          {#if phase === 'error'}
            <p class="text-[11px] text-red-300 leading-relaxed">
              {$t('update.installFailed')} {errorMessage}
            </p>
          {:else if phase === 'downloading'}
            <div class="flex items-center gap-2">
              <div class="flex-1 h-1.5 rounded bg-[var(--bg-hover)] overflow-hidden">
                <div class="h-full bg-[var(--accent)] transition-all" style="width: {percent}%"></div>
              </div>
              <span class="text-[10px] text-[var(--text-dim)] font-mono w-24 text-right">
                {$t('update.downloading', { percent })}
              </span>
            </div>
          {:else}
            <p class="text-[11px] text-[var(--accent-light)] animate-pulse">{$t('update.installing')}</p>
          {/if}
        </div>
      {/if}

      <div class="px-6 py-4 border-t border-[var(--border)] bg-[var(--bg-sidebar)] flex flex-col gap-3">
        <label class="flex items-center gap-2 text-[11px] text-[var(--text-muted)] cursor-pointer select-none">
          <input
            type="checkbox"
            checked={autoCheck}
            onchange={handleAutoCheckChange}
            class="accent-[var(--accent)] w-3.5 h-3.5"
          />
          {$t('update.autoCheck')}
        </label>

        <div class="flex items-center justify-between gap-2 flex-wrap">
          <button
            onclick={handleSkip}
            disabled={phase === 'downloading' || phase === 'installing'}
            class="text-[11px] text-[var(--text-dim)] hover:text-[var(--text-main)] transition disabled:opacity-40"
          >
            {$t('update.skip')}
          </button>

          <div class="flex items-center gap-2 flex-wrap">
            <button
              onclick={() => (isOpen = false)}
              disabled={phase === 'downloading' || phase === 'installing'}
              class="px-3 py-1.5 text-xs text-[var(--text-muted)] hover:text-[var(--text-main)] border border-[var(--border)] rounded-lg transition disabled:opacity-40"
            >
              {$t('update.later')}
            </button>
            <button
              onclick={handleOpenRelease}
              class="px-3 py-1.5 text-xs text-[var(--text-muted)] hover:text-[var(--text-main)] border border-[var(--border)] rounded-lg transition"
            >
              {$t('update.openRelease')}
            </button>
            {#if canInstall}
              <button
                onclick={handleInstallNow}
                disabled={phase === 'downloading' || phase === 'installing'}
                class="px-4 py-1.5 bg-[var(--accent)] hover:bg-[var(--accent-hover)] disabled:opacity-50 text-black text-xs font-semibold rounded-lg shadow-md transition"
              >
                {$t('update.installNow')}
              </button>
            {:else if update.download_url}
              <button
                onclick={handleDownloadPackage}
                class="px-4 py-1.5 bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-black text-xs font-semibold rounded-lg shadow-md transition"
              >
                {$t('update.downloadMatching')}
              </button>
            {/if}
          </div>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  @keyframes fadeIn {
    from { opacity: 0; transform: scale(0.97); }
    to { opacity: 1; transform: scale(1); }
  }
  .animate-fadeIn {
    animation: fadeIn 0.15s ease-out;
  }
  /* Compact Markdown for the dialog; the note preview styles are too large. */
  .release-notes :global(:where(h1, h2, h3, h4)) {
    color: var(--text-main);
    font-weight: 700;
    line-height: 1.3;
    margin: 1rem 0 0.4rem;
  }
  .release-notes :global(h1) { font-size: 0.95rem; }
  .release-notes :global(h2) { font-size: 0.85rem; }
  .release-notes :global(:where(h3, h4)) { font-size: 0.78rem; }
  .release-notes :global(:first-child) { margin-top: 0; }
  .release-notes :global(:where(p, ul, ol, pre, blockquote)) { margin: 0 0 0.5rem; }
  .release-notes :global(:where(ul, ol)) { padding-left: 1.1rem; }
  .release-notes :global(ul) { list-style: disc; }
  .release-notes :global(ol) { list-style: decimal; }
  .release-notes :global(li) { margin: 0.2rem 0; }
  .release-notes :global(strong) { color: var(--text-main); font-weight: 600; }
  .release-notes :global(a) { color: var(--accent-light); text-decoration: underline; }
  .release-notes :global(code) {
    font-family: ui-monospace, monospace;
    font-size: 0.95em;
    padding: 0 0.25rem;
    border-radius: 0.25rem;
    background: var(--bg-hover);
  }
  .release-notes :global(pre) { overflow-x: auto; padding: 0.5rem; border-radius: 0.375rem; background: var(--bg-hover); }
  .release-notes :global(pre code) { padding: 0; background: none; }
  .release-notes :global(blockquote) { border-left: 2px solid var(--border); padding-left: 0.6rem; }
  .release-notes :global(img) { max-width: 100%; height: auto; }
  .release-notes :global(.remote-image-blocked) {
    border: 1px dashed var(--border);
    border-radius: 0.375rem;
    padding: 0.25rem 0.5rem;
    color: var(--text-dim);
  }
</style>
