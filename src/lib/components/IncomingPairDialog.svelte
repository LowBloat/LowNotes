<script lang="ts">
  import { Link2 } from 'lucide-svelte';
  import type { PeerConfig } from '../types';
  import { networkAnswerPair } from '../api';
  import { t } from '$lib/i18n';
  import { dismissibleModal } from '$lib/modal-dismiss';

  let {
    request = null,
    onAnswer,
  } = $props<{
    request: { request_id: string; peer: PeerConfig } | null;
    onAnswer: () => void;
  }>();

  let answering = false;
  async function handleDecision(accept: boolean) {
    if (!request || answering) return;
    answering = true;
    try {
      await networkAnswerPair(request.request_id, accept);
    } catch (e) {
      console.error('Failed to answer pairing request:', e);
    } finally {
      answering = false;
      onAnswer();
    }
  }
</script>

{#if request}
  <div
    use:dismissibleModal={() => void handleDecision(false)}
    class="fixed inset-0 bg-black/80 z-50 flex items-center justify-center p-4 select-none"
    role="presentation"
  >
    <div
      class="bg-[var(--bg-card)] border-2 border-[var(--accent)] rounded-xl w-full max-w-md shadow-2xl p-6 flex flex-col gap-4"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <div class="flex items-center gap-3">
        <Link2 size={24} class="shrink-0 text-[var(--accent-light)]" />
        <div>
          <h3 class="text-sm font-bold text-[var(--text-main)]">{$t('incoming.title')}</h3>
          <p class="text-xs text-[var(--text-dim)]">{$t('incoming.subtitle')}</p>
        </div>
      </div>

      <div class="p-3 bg-[var(--bg-main)] border border-[var(--border)] rounded-lg flex flex-col gap-1">
        <span class="text-xs font-semibold text-[var(--text-main)]">{request.peer.name}</span>
        <span class="text-[11px] font-mono text-[var(--text-dim)] truncate">ID: {request.peer.endpoint_id}</span>
      </div>

      <p class="text-xs text-[var(--text-muted)] leading-relaxed">
        {$t('incoming.description')}
      </p>

      <div class="flex justify-end gap-2.5 pt-2">
        <button
          onclick={() => handleDecision(false)}
          class="px-4 py-2 text-xs font-medium text-red-400 hover:text-red-300 hover:bg-red-950/40 rounded-lg border border-red-900/40 transition"
        >
          {$t('incoming.reject')}
        </button>
        <button
          onclick={() => handleDecision(true)}
          class="px-4 py-2 text-xs font-semibold bg-[var(--accent)] hover:bg-[var(--accent-hover)] text-[var(--accent-contrast)] rounded-lg transition shadow-md"
        >
          {$t('incoming.accept')}
        </button>
      </div>
    </div>
  </div>
{/if}

