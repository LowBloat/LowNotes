<script lang="ts">
  import { Check, Copy, RefreshCw, X } from 'lucide-svelte';
  import type { NetworkEventPayload, PeerConfig } from '../types';
  import { networkRequestPair, networkRemovePeer, networkGetPairInfo, networkRegeneratePairCode } from '../api';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { onDestroy, onMount } from 'svelte';
  import { t, trError, ts } from '$lib/i18n';
  import { dismissibleModal } from '$lib/modal-dismiss';
  import { appConfirm } from '$lib/dialogs.svelte';
  let {
    isOpen = $bindable(false),
    pairCode = $bindable(''),
    endpointId = $bindable(''),
    vaultName = '',
    peers = [],
    onPeersChange,
  } = $props<{
    isOpen: boolean;
    pairCode?: string;
    endpointId?: string;
    vaultName: string;
    peers: PeerConfig[];
    onPeersChange?: () => void;
  }>();

  let activeTab = $state<'share' | 'join' | 'devices'>('share');
  let inputCode = $state('');
  let isConnecting = $state(false);
  let statusMessage = $state<{ type: 'success' | 'error' | 'info'; text: string } | null>(null);
  let copied = $state(false);

  let isLoadingCode = $state(false);
  let loadError = $state<string | null>(null);

  async function fetchPairInfo() {
    if (pairCode) return;
    isLoadingCode = true;
    loadError = null;
    try {
      const info = await networkGetPairInfo();
      if (info) {
        pairCode = info.pair_code;
        endpointId = info.endpoint_id;
      }
    } catch (e: any) {
      console.error('Failed to fetch pairing code:', e);
      loadError = trError(typeof e === 'string' ? e : e?.message || 'pair.fetchFailed');
    } finally {
      isLoadingCode = false;
    }
  }

  $effect(() => {
    if (isOpen && !pairCode) {
      fetchPairInfo();
    }
  });
  let regenerating = $state(false);
  async function regenerateCode() {
    regenerating = true;
    statusMessage = null;
    try {
      await networkRegeneratePairCode();
      copied = false;
      statusMessage = { type: 'info', text: ts('pair.regenerated') };
    } catch (e) {
      statusMessage = { type: 'error', text: trError(String(e)) };
    } finally {
      regenerating = false;
    }
  }

  function copyCode() {
    if (!pairCode) return;
    navigator.clipboard.writeText(pairCode);
    copied = true;
    setTimeout(() => {
      copied = false;
    }, 2500);
  }

  async function handleRequestPair() {
    if (!inputCode.trim()) return;

    isConnecting = true;
    statusMessage = { type: 'info', text: ts('pair.connecting') };

    try {
      await networkRequestPair(inputCode.trim());
      statusMessage = {
        type: 'info',
        text: ts('pair.requestSent'),
      };
      inputCode = '';
      onPeersChange?.();
    } catch (err: any) {
      statusMessage = {
        type: 'error',
        text: trError(typeof err === 'string' ? err : err.message || 'pair.requestFailed'),
      };
    } finally {
      isConnecting = false;
    }
  }

  async function handleRemove(peerId: string) {
    if (await appConfirm(ts('pair.confirmRemove'), { danger: true })) {
      try {
        await networkRemovePeer(peerId);
        onPeersChange?.();
      } catch (e) {
        console.error('Failed to remove peer:', e);
      }
    }
  }

  let unlistenError: UnlistenFn | null = null;

  onMount(async () => {
    unlistenError = await listen<NetworkEventPayload>('p2p:error', (event) => {
      if (event.payload.type === 'Error') {
        statusMessage = { type: 'error', text: trError(event.payload.message) };
      }
    });
  });

  onDestroy(() => {
    unlistenError?.();
  });
</script>

{#if isOpen}
  <!-- Backdrop -->
  <div
    use:dismissibleModal={() => (isOpen = false)}
    class="fixed inset-0 bg-black/70 backdrop-blur-sm z-50 flex items-center justify-center p-4 select-none animate-fadeIn"
    role="presentation"
  >
    <!-- Modal Dialog -->
    <div
      class="bg-[var(--bg-card)] border border-[var(--border)] rounded-xl w-full max-w-lg shadow-2xl flex flex-col overflow-hidden"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <!-- Header -->
      <div class="flex items-center justify-between px-6 py-4 border-b border-[var(--border)] bg-[var(--bg-sidebar)]">
        <div>
          <h2 class="text-base font-semibold text-[var(--text-main)]">{$t('pair.title')}</h2>
          <p class="text-xs text-[var(--text-dim)]">{$t('pair.currentVault')} <span class="text-[var(--text-muted)] font-medium">{vaultName}</span></p>
        </div>
        <button
          onclick={() => (isOpen = false)}
          title={$t('ai.close')}
          aria-label={$t('ai.close')}
          class="w-7 h-7 flex items-center justify-center rounded-md text-[var(--text-dim)] hover:text-[var(--text-main)] hover:bg-[var(--bg-hover)] transition"
        >
          <X size={16} />
        </button>
      </div>

      <!-- Tabs Navigation -->
      <div class="flex border-b border-[var(--border)] bg-[var(--bg-sidebar)] px-6">
        <button
          onclick={() => { activeTab = 'share'; statusMessage = null; }}
          class="py-2.5 px-3 text-xs font-medium border-b-2 transition {activeTab === 'share' ? 'border-[var(--accent)] text-[var(--accent-light)]' : 'border-transparent text-[var(--text-muted)] hover:text-[var(--text-main)]'}"
        >
          {$t('pair.tabShare')}
        </button>
        <button
          onclick={() => { activeTab = 'join'; statusMessage = null; }}
          class="py-2.5 px-3 text-xs font-medium border-b-2 transition {activeTab === 'join' ? 'border-[var(--accent)] text-[var(--accent-light)]' : 'border-transparent text-[var(--text-muted)] hover:text-[var(--text-main)]'}"
        >
          {$t('pair.tabJoin')}
        </button>
        <button
          onclick={() => { activeTab = 'devices'; statusMessage = null; }}
          class="py-2.5 px-3 text-xs font-medium border-b-2 transition {activeTab === 'devices' ? 'border-[var(--accent)] text-[var(--accent-light)]' : 'border-transparent text-[var(--text-muted)] hover:text-[var(--text-main)]'}"
        >
          {$t('pair.tabDevices', { count: peers.length })}
        </button>
      </div>

      <!-- Tab Content -->
      <div class="p-6 flex-1 overflow-y-auto">
        {#if activeTab === 'share'}
          <div class="flex flex-col gap-4">
            <p class="text-xs text-[var(--text-muted)] leading-relaxed">
              {$t('pair.shareDescription')} {$t('pair.singleUse')}
            </p>

            <div class="flex flex-col gap-1.5">
              <label for="p2p-code-display" class="text-xs font-medium text-[var(--text-dim)]">{$t('pair.inviteLabel')}</label>
              <div class="relative">
                <textarea
                  id="p2p-code-display"
                  readonly
                  value={pairCode || $t('pair.generating')}
                  rows="3"
                  class="w-full bg-[var(--bg-main)] border border-[var(--border)] rounded-lg p-3 text-xs font-mono text-[var(--text-main)] resize-none select-all focus:outline-none focus:border-[var(--accent)]"
                ></textarea>
              </div>
            </div>

            {#if loadError}
              <div class="p-3 rounded-lg text-xs leading-relaxed bg-red-950/40 border border-red-800/60 text-red-300 flex items-center justify-between">
                <span>{loadError}</span>
                <button
                  onclick={fetchPairInfo}
                  class="px-2.5 py-1 bg-red-900/60 hover:bg-red-800/80 rounded text-xs font-medium text-white transition"
                >
                  {$t('pair.retry')}
                </button>
              </div>
            {/if}

            <div class="flex justify-between items-center pt-2">
              <span class="text-[11px] text-[var(--text-dim)] font-mono truncate max-w-[280px]">
                {$t('pair.idLabel')} {endpointId ? (endpointId.length > 16 ? endpointId.slice(0, 16) + '...' : endpointId) : '...'}
              </span>
              <button
                onclick={regenerateCode}
                disabled={!pairCode || regenerating}
                class="ml-auto mr-2 px-3 py-2 border border-[var(--border)] hover:bg-[var(--bg-hover)] disabled:opacity-40 disabled:cursor-not-allowed text-[var(--text-main)] text-xs font-medium rounded-lg transition flex items-center gap-1.5"
              >
                <RefreshCw size={14} /> {$t('pair.regenerate')}
              </button>
              <button
                onclick={copyCode}
                disabled={!pairCode}
                class="px-4 py-2 bg-[var(--accent)] hover:bg-[var(--accent-hover)] disabled:opacity-40 disabled:cursor-not-allowed text-black text-xs font-semibold rounded-lg transition shadow-md flex items-center gap-1.5"
              >
                {#if copied}<Check size={15} />{:else}<Copy size={15} />{/if}
                {copied ? $t('pair.copied') : $t('pair.copy')}
              </button>
            </div>
            {#if statusMessage}
              <p role="status" class="text-xs {statusMessage.type === 'error' ? 'text-red-400' : 'text-[var(--text-muted)]'}">{statusMessage.text}</p>
            {/if}
          </div>
        {:else if activeTab === 'join'}
          <div class="flex flex-col gap-4">
            <p class="text-xs text-[var(--text-muted)] leading-relaxed">
              {$t('pair.joinDescription')}
            </p>

            <div class="flex flex-col gap-1.5">
              <label for="p2p-code-input" class="text-xs font-medium text-[var(--text-dim)]">{$t('pair.joinLabel')}</label>
              <textarea
                id="p2p-code-input"
                bind:value={inputCode}
                placeholder={$t('pair.joinPlaceholder')}
                rows="3"
                class="w-full bg-[var(--bg-main)] border border-[var(--border)] rounded-lg p-3 text-xs font-mono text-[var(--text-main)] resize-none focus:outline-none focus:border-[var(--accent)]"
              ></textarea>
            </div>

            {#if statusMessage}
              <div
                class="p-3 rounded-lg text-xs leading-relaxed {statusMessage.type === 'error' ? 'bg-red-950/40 border border-red-800/60 text-red-300' : statusMessage.type === 'success' ? 'bg-emerald-950/40 border border-emerald-800/60 text-emerald-300' : 'bg-amber-950/40 border border-amber-800/60 text-amber-300'}"
              >
                {statusMessage.text}
              </div>
            {/if}

            <div class="flex justify-end pt-2">
              <button
                onclick={handleRequestPair}
                disabled={isConnecting || !inputCode.trim()}
                class="px-4 py-2 bg-[var(--accent)] hover:bg-[var(--accent-hover)] disabled:opacity-50 text-black text-xs font-semibold rounded-lg transition shadow-md"
              >
                {isConnecting ? $t('pair.connectingButton') : $t('pair.requestPair')}
              </button>
            </div>
          </div>
        {:else if activeTab === 'devices'}
          <div class="flex flex-col gap-3">
            {#if peers.length === 0}
              <div class="text-center py-8">
                <p class="text-xs text-[var(--text-dim)]">{$t('pair.noDevices')}</p>
                <button
                  onclick={() => (activeTab = 'share')}
                  class="mt-2 text-xs text-[var(--accent-light)] hover:underline"
                >
                  {$t('pair.shareToStart')}
                </button>
              </div>
            {:else}
              <div class="flex flex-col gap-2">
                {#each peers as peer}
                  <div class="flex items-center justify-between p-3 rounded-lg bg-[var(--bg-main)] border border-[var(--border)]">
                    <div class="flex flex-col">
                      <span class="text-xs font-medium text-[var(--text-main)]">{peer.name}</span>
                      <span class="text-[11px] font-mono text-[var(--text-dim)]">ID: {peer.endpoint_id.slice(0, 20)}...</span>
                    </div>
                    <button
                      onclick={() => handleRemove(peer.endpoint_id)}
                      class="px-2.5 py-1 text-xs text-red-400 hover:text-red-300 hover:bg-red-950/30 rounded border border-transparent hover:border-red-900/50 transition"
                    >
                      {$t('pair.remove')}
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  @keyframes fadeIn {
    from { opacity: 0; transform: scale(0.98); }
    to { opacity: 1; transform: scale(1); }
  }
  .animate-fadeIn {
    animation: fadeIn 0.15s ease-out;
  }
</style>
